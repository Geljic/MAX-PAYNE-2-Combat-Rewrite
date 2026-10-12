//! From the sim's [`AnimRequest`] to joint positions, using the baked MP2 clips.
//!
//! No Bevy types here. The clips are in MP2's space (left-handed, +Z forward); every pose is
//! converted once to the sim's space (right-handed, forward -Z, character's right = +X) by
//! mirroring Z: positions negate z, quaternions negate x and y. Forward kinematics then runs in the
//! converted space, so a yaw of the body is a plain rotation about +Y.

use crate::anim::{AnimRequest, Clip as Want};
use crate::clips::{AnimSet, Clip, Joint, Pose, Quat, Vec3, quat_mul, quat_rotate, quat_slerp};
use crate::enemy::{EnemyAnim, Style, clip_candidates};
use crate::weapon::{WeaponClass, WeaponDef};

/// Bones the rig cares about, found by name once.
pub struct RigMap {
    /// Spine, Spine1, Neck: they share the aim twist.
    pub twist: [usize; 3],
    pub head: usize,
    pub r_gun: usize,
    pub l_gun: usize,
    /// Clavicle, upper arm, forearm, hand: the chains the aim solve bends.
    pub r_arm: [usize; 4],
    pub l_arm: [usize; 4],
    /// Bones the upper-body layer may overwrite (everything above the pelvis, arms included).
    pub upper: Vec<bool>,
    /// Capsules between joints: (from, to, radius).
    pub segs: Vec<(usize, usize, f32)>,
    /// Joint balls: (bone, radius).
    pub balls: Vec<(usize, f32)>,
}

impl RigMap {
    pub fn new(set: &AnimSet) -> Option<RigMap> {
        let i = |n: &str| set.bone_index(&format!("Bip01 {n}"));
        let spine = i("Spine")?;
        let spine1 = i("Spine1")?;
        let neck = i("Neck")?;
        let thighs = [i("L Thigh")?, i("R Thigh")?];
        let mut upper = vec![false; set.bones.len()];
        for (b, flag) in upper.iter_mut().enumerate() {
            let mut cur = Some(b);
            let (mut under_spine, mut under_thigh) = (false, false);
            while let Some(c) = cur {
                under_spine |= c == spine;
                under_thigh |= thighs.contains(&c);
                cur = set.bones[c].parent;
            }
            *flag = under_spine && !under_thigh;
        }
        let pairs: &[(&str, &str, f32)] = &[
            ("Pelvis", "Spine", 0.14),
            ("Spine", "Spine1", 0.15),
            ("Spine1", "Neck", 0.14),
            ("Neck", "Head", 0.055),
            ("L Clavicle", "L UpperArm", 0.06),
            ("L UpperArm", "L Forearm", 0.055),
            ("L Forearm", "L Hand", 0.045),
            ("R Clavicle", "R UpperArm", 0.06),
            ("R UpperArm", "R Forearm", 0.055),
            ("R Forearm", "R Hand", 0.045),
            ("Pelvis", "L Thigh", 0.09),
            ("L Thigh", "L Calf", 0.085),
            ("L Calf", "L Foot", 0.06),
            ("L Foot", "L Toe0", 0.045),
            ("Pelvis", "R Thigh", 0.09),
            ("R Thigh", "R Calf", 0.085),
            ("R Calf", "R Foot", 0.06),
            ("R Foot", "R Toe0", 0.045),
        ];
        let segs = pairs
            .iter()
            .filter_map(|(a, b, r)| Some((i(a)?, i(b)?, *r)))
            .collect();
        let ball_names: &[(&str, f32)] = &[
            ("Pelvis", 0.14),
            ("Spine", 0.15),
            ("Spine1", 0.14),
            ("L Thigh", 0.09),
            ("R Thigh", 0.09),
            ("L Toe0", 0.045),
            ("R Toe0", 0.045),
            ("Neck", 0.06),
            ("L UpperArm", 0.06),
            ("R UpperArm", 0.06),
            ("L Forearm", 0.05),
            ("R Forearm", 0.05),
            ("L Hand", 0.05),
            ("R Hand", 0.05),
            ("L Calf", 0.07),
            ("R Calf", 0.07),
            ("L Foot", 0.055),
            ("R Foot", 0.055),
        ];
        let balls = ball_names
            .iter()
            .filter_map(|(n, r)| Some((i(n)?, *r)))
            .collect();
        Some(RigMap {
            twist: [spine, spine1, neck],
            head: i("Head")?,
            r_gun: i("R Gun")?,
            l_gun: i("L Gun")?,
            r_arm: [
                i("R Clavicle")?,
                i("R UpperArm")?,
                i("R Forearm")?,
                i("R Hand")?,
            ],
            l_arm: [
                i("L Clavicle")?,
                i("L UpperArm")?,
                i("L Forearm")?,
                i("L Hand")?,
            ],
            upper,
            segs,
            balls,
        })
    }
}

/// The animation-name suffix of a weapon (`SHOOT` + suffix).
pub fn weapon_suffix(def: &WeaponDef) -> String {
    let n = def.name.to_uppercase();
    match def.class {
        _ if n.chars().all(|c| c.is_ascii_alphanumeric()) && !n.is_empty() => n,
        WeaponClass::Pistol => "BERETTA".into(),
        WeaponClass::DualPistol => "BERETTADUAL".into(),
        WeaponClass::Shotgun => "PUMPSHOTGUN".into(),
        WeaponClass::Smg => "MP5".into(),
        WeaponClass::DualSmg => "INGRAMDUAL".into(),
        WeaponClass::Rifle => "KALASHNIKOV".into(),
    }
}

/// Clip names to try for a lower / full-body request, and whether to hold its last frame.
fn lower_names(c: &Want) -> (Vec<String>, bool) {
    match c {
        Want::Prone(d) => (vec![Want::Shootdodge(*d).mp2_name()], true),
        Want::Death => (vec!["RAGDOLLDEATH".into(), "FALLINGDEATH".into()], false),
        Want::TargetFlinch(z, _) => (
            vec![
                if matches!(z, crate::HitZone::Head) {
                    "MEDIUMDAMAGE"
                } else {
                    "SMALLDAMAGE"
                }
                .into(),
            ],
            false,
        ),
        Want::TargetStagger(_) => (vec!["LARGEDAMAGE".into()], false),
        other => (vec![other.mp2_name()], false),
    }
}

/// Find a clip by slot name. `style` None looks the slot up as written (Max); `Some(style)` tries
/// the enemy order of [`clip_candidates`] (`SHARP/RUN..` → `ENEMY/RUN..` → `RUN..`).
fn find<'a>(set: &'a AnimSet, style: Option<Style>, slot: &str) -> Option<&'a Clip> {
    match style {
        None => set.clip(slot),
        Some(s) => clip_candidates(s, slot).iter().find_map(|n| set.clip(n)),
    }
}

/// The clip an enemy of `style` plays for `slot`, with the name it resolved to; None when none of
/// the candidates exists.
pub fn enemy_clip<'a>(set: &'a AnimSet, style: Style, slot: &str) -> Option<(String, &'a Clip)> {
    clip_candidates(style, slot)
        .into_iter()
        .find_map(|n| set.clip(&n).map(|c| (n, c)))
}

/// Sample a full-body request into `pose` (bind pose where the clip is silent).
fn sample_lower(set: &AnimSet, want: &Want, t: f32, pose: &mut Pose) {
    let (names, hold) = lower_names(want);
    let clip = names
        .iter()
        .find_map(|n| set.clip(n))
        .or_else(|| set.clip("STAND"));
    match clip {
        Some(c) => {
            let t = if hold { c.duration() } else { t };
            c.sample(set, t, pose);
        }
        None => *pose = set.bind_pose(),
    }
}

/// Layer the upper-body clips over `pose`: aim / fire / reload / switch, with the up and down
/// variants blended in by the aim pitch (MP2 ships SHOOTHIGH* and SHOOTLOW* for exactly this).
#[allow(clippy::too_many_arguments)]
fn layer_upper(
    set: &AnimSet,
    style: Option<Style>,
    map: &RigMap,
    upper: Want,
    t: f32,
    pitch: f32,
    suffix: &str,
    pose: &mut Pose,
) {
    let base_name = match upper {
        Want::Reload(_) => format!("RELOAD{suffix}"),
        Want::Switch(_) => "GETWEAPON".to_string(),
        _ => format!("SHOOT{suffix}"),
    };
    let (t, pitch_blend) = match upper {
        Want::Aim(_) => (0.0, true),
        Want::Fire(_) => (t, true),
        _ => (t, false),
    };
    let Some(base) =
        find(set, style, &base_name).or_else(|| find(set, style, &format!("SHOOT{suffix}")))
    else {
        return;
    };
    let mut layer = pose.clone();
    base.sample_into(t, &mut layer);
    if pitch_blend {
        let (word, w) = if pitch >= 0.0 {
            ("SHOOTHIGH", pitch)
        } else {
            ("SHOOTLOW", -pitch)
        };
        if let Some(alt) = find(set, style, &format!("{word}{suffix}")) {
            let w = (w / 0.8).clamp(0.0, 1.0);
            let mut other = pose.clone();
            alt.sample_into(t, &mut other);
            for &b in &alt.bones {
                let b = b as usize;
                layer.q[b] = quat_slerp(layer.q[b], other.q[b], w);
            }
        }
    }
    for &b in &base.bones {
        let b = b as usize;
        if map.upper[b] {
            pose.t[b] = layer.t[b];
            pose.q[b] = layer.q[b];
        }
    }
    // Keep the chest facing where the SHOOT clip faces it, whatever the legs do: the RUN* and
    // CROUCH clips turn and tilt the pelvis, and the spine would otherwise inherit that.
    let spine = map.twist[0];
    if let Some(pelvis) = set.bones[spine].parent {
        let want = set.model_space(&layer)[spine].rot;
        let have = set.model_space(pose)[pelvis].rot;
        pose.q[spine] = quat_mul(conj(have), want);
    }
}

/// Mirror a pose from MP2's left-handed space into the sim's.
fn convert(pose: &mut Pose) {
    for t in &mut pose.t {
        t[2] = -t[2];
    }
    for q in &mut pose.q {
        q[0] = -q[0];
        q[1] = -q[1];
    }
}

/// Rotate `bone` about its own joint by a rotation given in model space.
fn twist(set: &AnimSet, pose: &mut Pose, bone: usize, d: Quat) {
    let joints = set.model_space(pose);
    rotate_model(set, pose, &joints, bone, d);
}

/// [`twist`] with the joints already at hand (`joints` must match `pose`).
fn rotate_model(set: &AnimSet, pose: &mut Pose, joints: &[Joint], bone: usize, d: Quat) {
    let Some(p) = set.bones[bone].parent else {
        return;
    };
    let pr = joints[p].rot;
    pose.q[bone] = quat_mul(quat_mul(quat_mul(conj(pr), d), pr), pose.q[bone]);
}

fn axis_quat(axis: usize, angle: f32) -> Quat {
    let (s, c) = (angle * 0.5).sin_cos();
    let mut q = [0.0, 0.0, 0.0, c];
    q[axis] = s;
    q
}

// ---- small vector helpers (the core's math is Vec3-typed; joints are arrays) ----

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn len(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
fn norm(a: Vec3) -> Vec3 {
    let l = len(a);
    if l < 1e-9 {
        [0.0, 0.0, -1.0]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}
fn conj(q: Quat) -> Quat {
    [-q[0], -q[1], -q[2], q[3]]
}
const IDENT: Quat = [0.0, 0.0, 0.0, 1.0];

fn axis_angle(axis: Vec3, angle: f32) -> Quat {
    let a = norm(axis);
    let (s, c) = (angle * 0.5).sin_cos();
    [a[0] * s, a[1] * s, a[2] * s, c]
}

/// Shortest rotation taking unit `a` onto unit `b`.
fn from_to(a: Vec3, b: Vec3) -> Quat {
    let d = dot(a, b).clamp(-1.0, 1.0);
    if d < -0.9999 {
        // Opposite: any perpendicular axis will do.
        let p = if a[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        return axis_angle(cross(a, p), std::f32::consts::PI);
    }
    let c = cross(a, b);
    let w = 1.0 + d;
    let n = (dot(c, c) + w * w).sqrt();
    [c[0] / n, c[1] / n, c[2] / n, w / n]
}

/// Rotation angle of a unit quaternion (rad, 0..=PI).
fn quat_angle(q: Quat) -> f32 {
    2.0 * q[3].abs().clamp(0.0, 1.0).acos()
}

/// Angle between two directions (rad).
fn angle_between(a: Vec3, b: Vec3) -> f32 {
    dot(norm(a), norm(b)).clamp(-1.0, 1.0).acos()
}

// ---- aim solve ----

/// Which way a gun bone's barrel points in its own frame, after the Z mirror. Measured on the
/// SHOOT* aim poses: both gun bones sit axis-aligned with the body there, local +Z backwards.
pub const GUN_BARREL: Vec3 = [0.0, 0.0, -1.0];

/// Barrel origin and unit direction of a gun bone (character space).
pub fn barrel(joints: &[Joint], gun: usize) -> (Vec3, Vec3) {
    let j = joints[gun];
    (j.pos, norm(quat_rotate(j.rot, GUN_BARREL)))
}

/// Share of each solve step the clavicle takes before the shoulder does the rest. ESTIMATE: by eye.
const CLAVICLE_SHARE: f32 = 0.25;
/// How far the solve may turn the clavicle / upper arm away from the clip pose (rad). Past this the
/// gun stops following; the body (body_yaw follows the aim) closes the gap. ESTIMATE: by eye.
const CLAVICLE_MAX: f32 = 0.35;
const UPPER_ARM_MAX: f32 = 2.0;
/// Two-handed weapons: share of each correction the chest takes, and its cap (rad). ESTIMATE.
const CHEST_SHARE: f32 = 0.7;
const CHEST_MAX: f32 = 0.8;
/// Share of the aim yaw the spine takes in dives and prone, and its cap (rad). ESTIMATE: by eye.
const DIVE_TWIST_SHARE: f32 = 0.5;
const DIVE_TWIST_MAX: f32 = 0.8;
/// Solve passes. Each rotates the arm by the barrel's remaining miss; the barrel origin moves a
/// little as the arm turns, so a few passes are needed. 3 converges well under 0.1 degrees.
const AIM_PASSES: usize = 3;

/// Pull `pose.q[bone]` back towards `orig` so it is at most `max` rad away.
fn clamp_local(pose: &mut Pose, bone: usize, orig: Quat, max: f32) {
    let rel = quat_mul(conj(orig), pose.q[bone]);
    let a = quat_angle(rel);
    if a > max {
        pose.q[bone] = quat_slerp(orig, pose.q[bone], max / a);
    }
}

/// Turn one arm (`arm` = clavicle, upper arm, forearm, hand) so the barrel of `gun` passes through
/// `target`. Returns the remaining miss angle (rad).
///
/// With `chest`, that bone (Spine1) takes a share of each correction first, so both shoulders
/// travel with the gun: a two-handed weapon then stays within reach of the left hand.
fn aim_arm(
    set: &AnimSet,
    pose: &mut Pose,
    arm: [usize; 4],
    gun: usize,
    target: Vec3,
    chest: Option<usize>,
) -> f32 {
    let [clav, upper, _, _] = arm;
    let (orig_clav, orig_upper) = (pose.q[clav], pose.q[upper]);
    let orig_chest = chest.map(|c| pose.q[c]);
    let step = |pose: &mut Pose, bone: usize, share: f32| {
        let joints = set.model_space(pose);
        let (g, d) = barrel(&joints, gun);
        let want = norm(sub(target, g));
        let delta = quat_slerp(IDENT, from_to(d, want), share);
        rotate_model(set, pose, &joints, bone, delta);
    };
    for _ in 0..AIM_PASSES {
        if let (Some(c), Some(o)) = (chest, orig_chest) {
            step(pose, c, CHEST_SHARE);
            clamp_local(pose, c, o, CHEST_MAX);
        }
        step(pose, clav, CLAVICLE_SHARE);
        clamp_local(pose, clav, orig_clav, CLAVICLE_MAX);
        step(pose, upper, 1.0);
        clamp_local(pose, upper, orig_upper, UPPER_ARM_MAX);
    }
    let joints = set.model_space(pose);
    let (g, d) = barrel(&joints, gun);
    angle_between(d, sub(target, g))
}

/// Two-bone IK: put the hand of `arm` at `pos` with world rotation `rot`, bending the elbow in the
/// plane it already bends in.
fn reach(set: &AnimSet, pose: &mut Pose, arm: [usize; 4], pos: Vec3, rot: Quat) {
    let [_, upper, fore, hand] = arm;
    let joints = set.model_space(pose);
    let (s, e, w) = (joints[upper].pos, joints[fore].pos, joints[hand].pos);
    let (a, b) = (len(sub(e, s)), len(sub(w, e)));
    let c = len(sub(pos, s)).clamp((a - b).abs() + 1e-3, a + b - 1e-3);
    let interior = |x: Vec3, y: Vec3| angle_between(x, y);
    let cur = interior(sub(s, e), sub(w, e));
    let want = ((a * a + b * b - c * c) / (2.0 * a * b))
        .clamp(-1.0, 1.0)
        .acos();
    let mut n = cross(sub(e, s), sub(w, e));
    if len(n) < 1e-6 {
        n = cross(sub(e, s), sub(pos, s));
    }
    if len(n) > 1e-6 {
        rotate_model(set, pose, &joints, fore, axis_angle(n, cur - want));
    }
    let joints = set.model_space(pose);
    let w = joints[hand].pos;
    rotate_model(
        set,
        pose,
        &joints,
        upper,
        from_to(norm(sub(w, s)), norm(sub(pos, s))),
    );
    let joints = set.model_space(pose);
    let pr = joints[fore].rot;
    pose.q[hand] = quat_mul(conj(pr), rot);
}

/// The left hand's transform in the right gun's frame: (offset, rotation).
fn grip(joints: &[Joint], map: &RigMap) -> (Vec3, Quat) {
    let g = joints[map.r_gun];
    let h = joints[map.l_arm[3]];
    let gi = conj(g.rot);
    (quat_rotate(gi, sub(h.pos, g.pos)), quat_mul(gi, h.rot))
}

/// Whether the arms should follow the crosshair for this request: aiming or firing, in a state
/// where Max holds his gun up (standing, moving, crouched, airborne, shootdodging, prone). Off for
/// reloads, switches, spins, dodge rolls, get-ups, hit reactions and death.
pub fn aims(req: &AnimRequest) -> bool {
    aims_parts(&req.lower, req.upper)
}

fn aims_parts(lower: &Want, upper: Option<(Want, f32)>) -> bool {
    matches!(upper, Some((Want::Aim(_) | Want::Fire(_), _)))
        && matches!(
            lower,
            Want::Idle
                | Want::Run(_)
                | Want::Crouch
                | Want::CrouchWalk(_)
                | Want::Jump
                | Want::JumpForward
                | Want::Fall
                | Want::Land
                | Want::HardLand
                | Want::Shootdodge(_)
                | Want::Prone(_)
        )
}

/// Where the arms aim and how much: `target` in character space (feet at the origin, facing -Z,
/// i.e. world minus the feet, turned by -body_yaw), `weight` 0..=1 to fade the solve in.
#[derive(Clone, Copy, Debug)]
pub struct AimGoal {
    pub target: Vec3,
    pub weight: f32,
}

/// Joints (character space: feet on y = 0, forward -Z) for the player's request, with the arms
/// solved so the barrels pass through `aim` when given. Also returns the remaining miss per gun
/// (right, left; rad) for debugging.
pub fn player_joints(
    set: &AnimSet,
    map: &RigMap,
    req: &AnimRequest,
    def: &WeaponDef,
    aim: Option<AimGoal>,
) -> (Vec<Joint>, [Option<f32>; 2]) {
    let mut pose = set.bind_pose();
    sample_lower(set, &req.lower, req.lower_time, &mut pose);
    let look = Look {
        lower: req.lower,
        upper: req.upper,
        aim_yaw: req.aim_yaw,
        aim_pitch: req.aim_pitch,
    };
    layered(set, map, pose, None, &look, def, aim)
}

/// What the upper body does on top of a sampled lower pose.
struct Look {
    lower: Want,
    upper: Option<(Want, f32)>,
    aim_yaw: f32,
    aim_pitch: f32,
}

/// Upper-body layer, aim twist and arm solve over an already sampled (MP2-space) `pose`. `style`
/// None looks clips up as Max's; `Some` uses an enemy's candidate order.
fn layered(
    set: &AnimSet,
    map: &RigMap,
    mut pose: Pose,
    style: Option<Style>,
    req: &Look,
    def: &WeaponDef,
    aim: Option<AimGoal>,
) -> (Vec<Joint>, [Option<f32>; 2]) {
    // The aim layer assumes an upright chest. Dives and prone poses carry their own arms (the
    // SHOOTDODGE clips hold both guns forward), so there only the arm solve below runs; twisting a
    // horizontal torso about the vertical would be wrong.
    let upright = matches!(
        req.lower,
        Want::Idle
            | Want::Run(_)
            | Want::Crouch
            | Want::CrouchWalk(_)
            | Want::Jump
            | Want::JumpForward
            | Want::Fall
            | Want::Land
            | Want::HardLand
    );
    let suffix = weapon_suffix(def);
    if upright && let Some((c, t)) = req.upper {
        layer_upper(set, style, map, c, t, req.aim_pitch, &suffix, &mut pose);
    }
    convert(&mut pose);
    if upright {
        // Body language: yaw spread over the spine and neck so the whole torso turns; pitch partly
        // from the clip blend above, a little here. The arm solve then puts the barrel on target.
        let w = [0.35, 0.4, 0.25];
        for (k, &b) in map.twist.iter().enumerate() {
            let d = quat_mul(
                axis_quat(1, req.aim_yaw * w[k]),
                axis_quat(0, req.aim_pitch * 0.35 * w[k]),
            );
            twist(set, &mut pose, b, d);
        }
    }
    let mut miss = [None, None];
    let aiming = aims_parts(&req.lower, req.upper);
    if !upright && aiming {
        // Dives and prone: the torso is horizontal, so the aim yaw becomes a twist about the
        // spine's own long axis (Biped bones run along local +X), clamped, before the arms solve.
        // Upright, the same twist about +X is the yaw twist above.
        let total = (req.aim_yaw * DIVE_TWIST_SHARE).clamp(-DIVE_TWIST_MAX, DIVE_TWIST_MAX);
        let w = [0.4, 0.4, 0.2];
        for (k, &b) in map.twist.iter().enumerate() {
            let joints = set.model_space(&pose);
            let axis = quat_rotate(joints[b].rot, [1.0, 0.0, 0.0]);
            rotate_model(set, &mut pose, &joints, b, axis_angle(axis, total * w[k]));
        }
    }
    let Some(goal) = aim.filter(|g| g.weight > 0.0 && aiming) else {
        return (set.model_space(&pose), miss);
    };
    let two_handed = def.two_handed && !def.dual;
    // Where the left hand holds a two-handed weapon, relative to the gun: as the SHOOT clip has it.
    let mut grip_rel = two_handed.then(|| grip(&set.model_space(&pose), map));
    if two_handed
        && !upright
        && let Some(c) = find(set, style, &format!("SHOOT{suffix}"))
    {
        // The dive clips hold two pistols at arm's length; a rifle there would be out of the left
        // hand's reach. Hold it as in the standing aim instead (arms keep their standing
        // orientation relative to the character, hung from the dive's shoulders), like a prone
        // rifleman, and let the solve below turn it onto the target.
        let mut shoot = set.bind_pose();
        c.sample(set, 0.0, &mut shoot);
        convert(&mut shoot);
        let js = set.model_space(&shoot);
        grip_rel = Some(grip(&js, map));
        let joints = set.model_space(&pose);
        for arm in [map.r_arm, map.l_arm] {
            let [clav, upper, fore, hand] = arm;
            pose.q[upper] = quat_mul(conj(joints[clav].rot), js[upper].rot);
            pose.q[fore] = shoot.q[fore];
            pose.q[hand] = shoot.q[hand];
        }
        for gun in [map.r_gun, map.l_gun] {
            pose.q[gun] = shoot.q[gun];
            pose.t[gun] = shoot.t[gun];
        }
    }
    let before = pose.clone();
    let chest = grip_rel.is_some().then_some(map.twist[1]);
    miss[0] = Some(aim_arm(
        set,
        &mut pose,
        map.r_arm,
        map.r_gun,
        goal.target,
        chest,
    ));
    if def.dual {
        miss[1] = Some(aim_arm(
            set,
            &mut pose,
            map.l_arm,
            map.l_gun,
            goal.target,
            None,
        ));
    } else if let Some((off, rot)) = grip_rel {
        let joints = set.model_space(&pose);
        let g = joints[map.r_gun];
        let mut pos = add(g.pos, quat_rotate(g.rot, off));
        // Out of reach (extreme twists in a dive): slide the hand back along the fore-end until the
        // arm reaches, but never behind the right hand.
        let [_, u, f, h] = map.l_arm;
        let sh = joints[u].pos;
        let max = 0.98 * (len(sub(joints[f].pos, sh)) + len(sub(joints[h].pos, joints[f].pos)));
        let (_, dir) = barrel(&joints, map.r_gun);
        let to = sub(pos, sh);
        if len(to) > max {
            // |to - dir t| = max for the smallest t >= 0.
            let b = dot(to, dir);
            let disc = b * b - (dot(to, to) - max * max);
            let along = dot(sub(pos, g.pos), dir).max(0.0);
            if disc >= 0.0 {
                let t = (b - disc.sqrt()).clamp(0.0, along);
                pos = sub(pos, [dir[0] * t, dir[1] * t, dir[2] * t]);
            }
        }
        reach(set, &mut pose, map.l_arm, pos, quat_mul(g.rot, rot));
    }
    if goal.weight < 1.0 {
        for &b in map.r_arm.iter().chain(&map.l_arm).chain(&map.twist[1..2]) {
            pose.q[b] = quat_slerp(before.q[b], pose.q[b], goal.weight);
        }
        miss = [None, None];
    }
    (set.model_space(&pose), miss)
}

/// Joints for an enemy: its body clip (looped or clamped at `anim.t`), then, while it aims, the
/// SHOOT layer, twist and arm solve with the barrels on `anim.aim` (given in character space as
/// `goal`; the yaw and pitch from the chest follow from it). Slots that no clip answers to fall
/// back to STAND.
pub fn enemy_joints(
    set: &AnimSet,
    map: &RigMap,
    anim: &EnemyAnim,
    def: &WeaponDef,
    goal: Option<AimGoal>,
) -> Vec<Joint> {
    let mut pose = set.bind_pose();
    let clip = enemy_clip(set, anim.style, anim.slot)
        .or_else(|| enemy_clip(set, anim.style, "STAND"))
        .map(|(_, c)| c);
    match clip {
        Some(c) => {
            let d = c.duration();
            let t = if anim.looping && d > 0.0 {
                anim.t % d
            } else {
                anim.t.min(d)
            };
            c.sample(set, t, &mut pose);
        }
        None => pose = set.bind_pose(),
    }
    // Hit reactions play as authored; the rest hold the gun up while aiming.
    let lower = match anim.slot {
        "CROUCH" => Want::Crouch,
        s if s.ends_with("DAMAGE") || s.starts_with("KNOCK") => {
            Want::Flinch(crate::anim::HitSide::Front)
        }
        _ => Want::Idle,
    };
    let chest_h = if matches!(lower, Want::Crouch) {
        0.95
    } else {
        1.4
    };
    let (aim_yaw, aim_pitch) = goal.map_or((0.0, 0.0), |g| {
        let d = crate::math::Vec3::new(g.target[0], g.target[1] - chest_h, g.target[2]);
        (crate::math::dir_yaw(d), crate::math::dir_pitch(d))
    });
    let look = Look {
        lower,
        upper: goal.map(|_| (Want::Aim(def.class), 0.0)),
        aim_yaw,
        aim_pitch,
    };
    layered(set, map, pose, Some(anim.style), &look, def, goal).0
}

/// Joints for a plain clip (targets).
pub fn clip_joints(set: &AnimSet, want: &Want, t: f32) -> Vec<Joint> {
    let mut pose = set.bind_pose();
    sample_lower(set, want, t, &mut pose);
    convert(&mut pose);
    set.model_space(&pose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::MoveDir;
    use crate::weapon::default_roster;

    /// The real clips, when the tools have generated them (the crate builds without the game).
    fn load() -> Option<(AnimSet, RigMap)> {
        let bytes = std::fs::read("assets/generated/anims.bin").ok()?;
        let set = AnimSet::load(&bytes).ok()?;
        let map = RigMap::new(&set)?;
        Some((set, map))
    }

    fn request(lower: Want, t: f32, upper: Want, aim_yaw: f32, aim_pitch: f32) -> AnimRequest {
        AnimRequest {
            lower,
            lower_time: t,
            upper: Some((upper, 0.0)),
            body_yaw: 0.0,
            aim_yaw,
            aim_pitch,
            speed: 0.0,
            blend: 0.15,
            time_scale: 1.0,
        }
    }

    #[test]
    fn spine_bones_run_along_local_x() {
        let Some((set, map)) = load() else { return };
        let mut pose = set.bind_pose();
        set.clip("STAND").unwrap().sample(&set, 0.0, &mut pose);
        convert(&mut pose);
        let j = set.model_space(&pose);
        for &b in &map.twist {
            let x = quat_rotate(j[b].rot, [1.0, 0.0, 0.0]);
            assert!(x[1] > 0.9, "{} x axis {x:?}", set.bones[b].name);
        }
    }

    /// Every aiming state, every hand: the barrel line passes through the aim point within 1°,
    /// the right gun stays on the right, and a two-handed weapon keeps the left hand on its grip.
    #[test]
    fn barrels_land_on_the_aim_point() {
        let Some((set, map)) = load() else { return };
        let roster = default_roster();
        let lowers = [
            (Want::Idle, 0.0, 1.4),
            (Want::Run(MoveDir::Forward), 0.3, 1.4),
            (Want::Run(MoveDir::Back), 0.3, 1.4),
            (Want::Run(MoveDir::Left), 0.3, 1.4),
            (Want::Run(MoveDir::Right), 0.3, 1.4),
            (Want::Crouch, 0.0, 0.95),
            (Want::CrouchWalk(MoveDir::Forward), 0.2, 0.95),
            (Want::Shootdodge(MoveDir::Forward), 0.6, 0.6),
            (Want::Shootdodge(MoveDir::Left), 0.6, 0.6),
            (Want::Prone(MoveDir::Forward), 0.0, 0.4),
            (Want::Prone(MoveDir::Back), 0.0, 0.4),
        ];
        let mut worst = 0.0f32;
        for name in [
            "Beretta",
            "BerettaDual",
            "Kalashnikov",
            "MP5",
            "PumpShotgun",
        ] {
            let def = roster.iter().find(|d| d.name == name).unwrap();
            for &(lower, t, h) in &lowers {
                for yaw in [-0.5f32, 0.0, 0.5] {
                    for pitch in [-0.5f32, 0.0, 0.5] {
                        for upper in [Want::Aim(def.class), Want::Fire(def.class)] {
                            let req = request(lower, t, upper, yaw, pitch);
                            let d = crate::math::aim_dir(yaw, pitch);
                            let target = [d.x * 6.0, h + d.y * 6.0, d.z * 6.0];
                            let goal = AimGoal {
                                target,
                                weight: 1.0,
                            };
                            let (j, miss) = player_joints(&set, &map, &req, def, Some(goal));
                            let tag = format!("{name} {lower:?} {upper:?} yaw {yaw} pitch {pitch}");
                            let guns: &[usize] = if def.dual {
                                &[map.r_gun, map.l_gun]
                            } else {
                                &[map.r_gun]
                            };
                            for (k, &g) in guns.iter().enumerate() {
                                let (o, dir) = barrel(&j, g);
                                let err = angle_between(dir, sub(target, o)).to_degrees();
                                worst = worst.max(err);
                                assert!(err < 1.0, "{tag}: gun {k} misses by {err:.2}°");
                                assert!((miss[k].unwrap().to_degrees() - err).abs() < 0.01);
                            }
                            if def.dual && yaw == 0.0 && matches!(lower, Want::Idle) {
                                assert!(
                                    j[map.r_gun].pos[0] > j[map.l_gun].pos[0],
                                    "{tag}: hands swapped"
                                );
                            }
                            if def.two_handed {
                                // The left hand rides on the weapon: near the barrel line, ahead
                                // of the right hand.
                                let (o, dir) = barrel(&j, map.r_gun);
                                let h = sub(j[map.l_arm[3]].pos, o);
                                let along = dot(h, dir);
                                let off =
                                    len(sub(h, [dir[0] * along, dir[1] * along, dir[2] * along]));
                                assert!(
                                    along > 0.0 && off < 0.1,
                                    "{tag}: left hand off the gun ({along:.2} along, {off:.2} off)"
                                );
                            }
                        }
                    }
                }
            }
        }
        println!("worst barrel miss {worst:.3}°");
    }

    /// Enemy slots resolve in the order S/ -> ENEMY/ -> bare, and every slot the AI asks for has a
    /// clip for every style (so no enemy ever falls back to STAND silently).
    #[test]
    fn enemy_clips_resolve_in_candidate_order() {
        let Some((set, _)) = load() else { return };
        let (name, _) = enemy_clip(&set, Style::Sharp, "RUNFORWARD").unwrap();
        assert_eq!(name, "SHARP/RUNFORWARD");
        let (name, _) = enemy_clip(&set, Style::Plain, "RUNFORWARD").unwrap();
        assert!(name == "ENEMY/RUNFORWARD" || name == "RUNFORWARD", "{name}");
        assert!(enemy_clip(&set, Style::Sharp, "NOSUCHSLOT").is_none());
        let slots = [
            "STAND",
            "CROUCH",
            "CONFUSED",
            "SMALLDAMAGE",
            "LARGEDAMAGE",
            "RUNFORWARD",
            "RUNBACKWARD",
            "RUNLEFT",
            "RUNRIGHT",
            "WALKFORWARD",
            "WALKLEFT",
            "SHOOTBERETTA",
        ];
        for style in [Style::Plain, Style::Normal, Style::Sharp] {
            for slot in slots {
                assert!(
                    enemy_clip(&set, style, slot).is_some(),
                    "{style:?} {slot}: none of {:?} exists",
                    clip_candidates(style, slot)
                );
            }
        }
    }

    /// Reloads, switches, rolls, get-ups and hit reactions play as authored: no arm solve.
    #[test]
    fn solve_is_off_outside_aiming() {
        let Some((set, map)) = load() else { return };
        let def = &default_roster()[0];
        let goal = Some(AimGoal {
            target: [3.0, 1.4, -3.0],
            weight: 1.0,
        });
        for (lower, upper) in [
            (Want::Idle, Want::Reload(def.class)),
            (Want::Idle, Want::Switch(def.class)),
            (Want::GetUp(MoveDir::Forward), Want::Aim(def.class)),
            (Want::Dodge(MoveDir::Left), Want::Aim(def.class)),
            (
                Want::Flinch(crate::anim::HitSide::Front),
                Want::Aim(def.class),
            ),
            (Want::Spin, Want::Aim(def.class)),
        ] {
            let req = request(lower, 0.1, upper, 0.3, 0.0);
            let (with, miss) = player_joints(&set, &map, &req, def, goal);
            let (without, _) = player_joints(&set, &map, &req, def, None);
            assert_eq!(miss, [None, None]);
            assert_eq!(
                with[map.r_gun].pos, without[map.r_gun].pos,
                "{lower:?} {upper:?}"
            );
        }
    }
}
