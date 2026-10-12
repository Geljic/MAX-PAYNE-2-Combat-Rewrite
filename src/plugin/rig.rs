//! Drawing: the capsule rig for Max, targets and corpses, bullet streaks, flashes, debug gizmos.
//!
//! A [`Figure`] is a pool of unit meshes (cylinders, balls, boxes) that is re-posed every frame from
//! a [`Shape`]: capsules between joints, balls, and weapon boxes. Poses come from [`super::pose`]
//! when `anims.bin` is loaded, and from the sim's own skeleton otherwise.

use bevy::prelude::*;

use super::pose::{self, RigMap};
use super::{CameraRig, PayneAnims, PayneEvent, PayneSim, PayneUi, bv};
use crate::clips::Joint;
use crate::target::TargetState;
use crate::weapon::{Hand, WeaponDef};
use crate::world::{Aabb, TargetId};

#[derive(Resource)]
pub struct PayneRigMap(pub Option<RigMap>);

/// Where the guns are, for muzzle flashes.
#[derive(Resource, Default)]
pub struct GunPoints {
    pub right: Option<Vec3>,
    pub left: Option<Vec3>,
    /// Barrel origin and unit direction of each drawn gun (world), for the debug gizmos.
    pub barrels: Vec<(Vec3, Vec3)>,
}

/// The arm solve's smoothed goal: aim direction from the chest and fade-in weight.
#[derive(Default)]
pub struct AimSmooth {
    dir: Option<Vec3>,
    weight: f32,
}

/// Time constant of the aim smoothing (s): short enough to trail a mouse sweep by about a frame,
/// long enough that a crosshair jumping between near and far hits does not snap the arm.
/// ESTIMATE: by eye.
const AIM_SMOOTH: f32 = 0.02;
/// Seconds to fade the arm solve in when aiming resumes (after a reload, switch, roll or get-up the
/// gun comes up onto the target instead of snapping there). ESTIMATE: by eye.
const AIM_FADE: f32 = 0.12;

#[derive(Resource, Default)]
pub struct Streaks(Vec<Entity>);

#[derive(Resource)]
pub struct RigAssets {
    cyl: Handle<Mesh>,
    ball: Handle<Mesh>,
    cube: Handle<Mesh>,
    streak: Handle<StandardMaterial>,
    /// Enemy bullets: a hot orange-red, to tell them from Max's.
    enemy_streak: Handle<StandardMaterial>,
    flash: Handle<StandardMaterial>,
    spark: Handle<StandardMaterial>,
    gun: Handle<StandardMaterial>,
}

/// Capsules, balls and boxes to show, in world space.
#[derive(Component, Default)]
pub struct Shape {
    segs: Vec<(Vec3, Vec3, f32)>,
    balls: Vec<(Vec3, f32)>,
    /// Weapon boxes: translation, rotation, size.
    boxes: Vec<(Vec3, Quat, Vec3)>,
}

#[derive(Component)]
pub struct Figure {
    mat: Handle<StandardMaterial>,
    cyl: Vec<Entity>,
    ball: Vec<Entity>,
    cube: Vec<Entity>,
    /// Hit flash, 1 = full.
    flash: f32,
}

#[derive(Component)]
pub struct PlayerFigure;
#[derive(Component)]
pub struct TargetFigure(pub TargetId);
#[derive(Component)]
pub struct CorpseFigure(pub usize);

/// A short-lived effect (muzzle flash, spark).
#[derive(Component)]
pub struct Fx(f32);

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut ui: ResMut<PayneUi>,
) {
    // PAYNE_DEBUG=1 starts with the F1 gizmos on (for scripted screenshots).
    ui.debug |= std::env::var_os("PAYNE_DEBUG").is_some();
    let unlit = |c: Color| StandardMaterial {
        base_color: c,
        unlit: true,
        ..default()
    };
    let assets = RigAssets {
        cyl: meshes.add(Cylinder::new(1.0, 1.0)),
        ball: meshes.add(Sphere::new(1.0)),
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        streak: mats.add(unlit(Color::srgb(1.0, 0.92, 0.45))),
        enemy_streak: mats.add(unlit(Color::srgb(1.0, 0.28, 0.08))),
        flash: mats.add(unlit(Color::srgb(1.0, 0.75, 0.3))),
        spark: mats.add(unlit(Color::srgb(1.0, 0.95, 0.7))),
        gun: mats.add(StandardMaterial {
            base_color: Color::srgb(0.07, 0.07, 0.09),
            metallic: 0.6,
            perceptual_roughness: 0.4,
            ..default()
        }),
    };
    let player = new_figure(&mut commands, &mut mats, Color::srgb(0.2, 0.26, 0.38));
    commands.entity(player).insert(PlayerFigure);
    commands.insert_resource(assets);
}

fn new_figure(commands: &mut Commands, mats: &mut Assets<StandardMaterial>, c: Color) -> Entity {
    commands
        .spawn((
            Transform::default(),
            Visibility::default(),
            Shape::default(),
            Figure {
                mat: mats.add(StandardMaterial {
                    base_color: c,
                    perceptual_roughness: 0.8,
                    ..default()
                }),
                cyl: Vec::new(),
                ball: Vec::new(),
                cube: Vec::new(),
                flash: 0.0,
            },
        ))
        .id()
}

/// Keep one figure per live target and per corpse.
pub fn sync_figures(
    mut commands: Commands,
    mut mats: ResMut<Assets<StandardMaterial>>,
    sim: Res<PayneSim>,
    targets: Query<(Entity, &TargetFigure)>,
    corpses: Query<(Entity, &CorpseFigure)>,
) {
    for t in &sim.0.targets {
        if !targets.iter().any(|(_, f)| f.0 == t.id) {
            let colour = t.ai.as_ref().map_or(Color::srgb(0.62, 0.24, 0.2), |ai| {
                suit_colour(ai.skin.style)
            });
            let e = new_figure(&mut commands, &mut mats, colour);
            commands.entity(e).insert(TargetFigure(t.id));
        }
    }
    for (e, f) in &targets {
        if !sim.0.targets.iter().any(|t| t.id == f.0) {
            commands.entity(e).despawn();
        }
    }
    for i in 0..sim.0.corpses.len() {
        if !corpses.iter().any(|(_, f)| f.0 == i) {
            let e = new_figure(&mut commands, &mut mats, Color::srgb(0.36, 0.12, 0.12));
            commands.entity(e).insert(CorpseFigure(i));
        }
    }
    for (e, f) in &corpses {
        if f.0 >= sim.0.corpses.len() {
            commands.entity(e).despawn();
        }
    }
}

/// Suit colour of an enemy by animation style, so gunmen read as different from the red paper
/// targets: dark greys, the Sharp (black ops) ones nearly black.
fn suit_colour(style: crate::enemy::Style) -> Color {
    use crate::enemy::Style::*;
    match style {
        Plain => Color::srgb(0.27, 0.26, 0.25),
        Normal => Color::srgb(0.2, 0.22, 0.27),
        Sharp => Color::srgb(0.08, 0.08, 0.09),
    }
}

/// Segments and balls for a posed 86-bone skeleton, placed in the world.
fn shape_from_joints(
    map: &RigMap,
    joints: &[Joint],
    pos: Vec3,
    yaw: f32,
    guns: Option<&WeaponDef>,
    gun_points: &mut GunPoints,
) -> Shape {
    let turn = Quat::from_rotation_y(yaw);
    let at = |b: usize| pos + turn * Vec3::from_array(joints[b].pos);
    let mut s = Shape::default();
    for &(a, b, r) in &map.segs {
        s.segs.push((at(a), at(b), r));
    }
    for &(b, r) in &map.balls {
        s.balls.push((at(b), r));
    }
    // The head bone sits at the base of the skull; the head itself is a ball further along its axis.
    let hr = turn * rotate(joints[map.head].rot, Vec3::X);
    s.balls.push((at(map.head) + hr * 0.1, 0.115));
    if let Some(def) = guns {
        let size = gun_size(def);
        let hands: &[(usize, Hand)] = if def.dual {
            &[(map.r_gun, Hand::Right), (map.l_gun, Hand::Left)]
        } else {
            &[(map.r_gun, Hand::Right)]
        };
        for &(g, hand) in hands {
            let j = joints[g];
            let rot = turn * Quat::from_xyzw(j.rot[0], j.rot[1], j.rot[2], j.rot[3]);
            let p = at(g);
            // The box's long axis (local -Z) is laid along the gun bone's barrel axis.
            let fix = Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::from_array(pose::GUN_BARREL));
            let q = rot * fix;
            gun_points.barrels.push((p, q * Vec3::NEG_Z));
            let centre = p + q * Vec3::NEG_Z * (size.z * 0.4);
            s.boxes.push((centre, q, size));
            match hand {
                Hand::Right => gun_points.right = Some(p + q * Vec3::NEG_Z * size.z * 0.9),
                Hand::Left => gun_points.left = Some(p + q * Vec3::NEG_Z * size.z * 0.9),
            }
        }
    }
    s
}

fn gun_size(def: &WeaponDef) -> Vec3 {
    use crate::weapon::WeaponClass::*;
    match def.class {
        Pistol | DualPistol => Vec3::new(0.035, 0.1, 0.24),
        Smg | DualSmg => Vec3::new(0.05, 0.13, 0.36),
        Shotgun => Vec3::new(0.05, 0.1, 0.8),
        Rifle => Vec3::new(0.05, 0.12, 0.85),
    }
}

fn rotate(q: [f32; 4], v: Vec3) -> Vec3 {
    Quat::from_xyzw(q[0], q[1], q[2], q[3]) * v
}

/// Capsules from the sim's own skeleton (no `anims.bin`): joints at rest around `pos`.
fn shape_from_skeleton(
    parents: &[(Option<usize>, f32)],
    head: Option<usize>,
    points: &[Vec3],
    radius_scale: f32,
) -> Shape {
    let mut s = Shape::default();
    for (i, &(parent, r)) in parents.iter().enumerate() {
        if let Some(p) = parent {
            s.segs.push((points[p], points[i], r * radius_scale));
        }
        s.balls.push((points[i], r * radius_scale));
    }
    if let Some(h) = head {
        s.balls.push((points[h], 0.12));
    }
    s
}

fn skeleton_parents(sim: &crate::Sim) -> Vec<(Option<usize>, f32)> {
    sim.skeleton
        .joints
        .iter()
        .map(|j| (j.parent, j.radius))
        .collect()
}

pub fn pose_player(
    time: Res<Time>,
    sim: Res<PayneSim>,
    anims: Res<PayneAnims>,
    map: Res<PayneRigMap>,
    mut guns: ResMut<GunPoints>,
    mut smooth: Local<AimSmooth>,
    mut q: Query<(&mut Shape, &mut Visibility), With<PlayerFigure>>,
) {
    let Ok((mut shape, mut vis)) = q.single_mut() else {
        return;
    };
    let p = &sim.0.player;
    guns.right = None;
    guns.left = None;
    guns.barrels.clear();
    if !p.alive() {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Inherited;
    let def = &sim.0.weapons[p.weapon().def];
    let req = sim.0.anim();
    *shape = match (&anims.0, &map.0) {
        (Some(set), Some(m)) => {
            let goal = aim_goal(&sim.0, &req, time.delta_secs(), &mut smooth);
            let (joints, miss) = pose::player_joints(set, m, &req, def, goal);
            if std::env::var_os("PAYNE_AIM_LOG").is_some() {
                let deg =
                    |m: Option<f32>| m.map_or("-".into(), |r| format!("{:.2}", r.to_degrees()));
                info!(
                    "aim t={:.3} lower={:?} upper={:?} aim_yaw={:.2} pitch={:.2} weapon={} miss_deg R={} L={}",
                    sim.0.time,
                    req.lower,
                    req.upper.map(|u| u.0),
                    req.aim_yaw,
                    req.aim_pitch,
                    def.name,
                    deg(miss[0]),
                    deg(miss[1]),
                );
            }
            shape_from_joints(m, &joints, bv(p.pos), req.body_yaw, Some(def), &mut guns)
        }
        _ => {
            let pts = rest_points(&sim.0, bv(p.pos), req.body_yaw);
            shape_from_skeleton(
                &skeleton_parents(&sim.0),
                sim.0.skeleton.joint("Bip01 Head"),
                &pts,
                0.8,
            )
        }
    };
}

/// The arm solve's goal for this frame: the sim's aim point in character space, reached along a
/// lightly smoothed direction from the chest, and the solve weight faded in and out by state.
fn aim_goal(
    sim: &crate::Sim,
    req: &crate::anim::AnimRequest,
    dt: f32,
    s: &mut AimSmooth,
) -> Option<pose::AimGoal> {
    let p = &sim.player;
    let chest = bv(p.chest(&sim.tuning));
    let to = bv(p.aim_target) - chest;
    let dist = to.length().max(1e-3);
    let raw = to / dist;
    let k = 1.0 - (-dt / AIM_SMOOTH).exp();
    let dir = match s.dir {
        Some(d) if dt > 0.0 => d.lerp(raw, k).normalize_or(raw),
        Some(d) => d,
        None => raw,
    };
    s.dir = Some(dir);
    // Off at once when aiming stops: the upper layer switches clip (reload, switch) or the body
    // plays as authored, so there is nothing to fade from.
    if !pose::aims(req) {
        s.weight = 0.0;
        return None;
    }
    s.weight = (s.weight + dt / AIM_FADE).min(1.0);
    let world = chest + dir * dist;
    let local = Quat::from_rotation_y(-req.body_yaw) * (world - bv(p.pos));
    Some(pose::AimGoal {
        target: local.to_array(),
        weight: s.weight,
    })
}

fn rest_points(sim: &crate::Sim, feet: Vec3, yaw: f32) -> Vec<Vec3> {
    let turn = Quat::from_rotation_y(yaw);
    sim.skeleton
        .joints
        .iter()
        .map(|j| feet + turn * bv(j.rest))
        .collect()
}

pub fn pose_targets(
    sim: Res<PayneSim>,
    anims: Res<PayneAnims>,
    map: Res<PayneRigMap>,
    mut q: Query<(&TargetFigure, &mut Shape, &mut Visibility)>,
    mut warned: Local<std::collections::HashSet<String>>,
) {
    let mut unused = GunPoints::default();
    for (f, mut shape, mut vis) in &mut q {
        let Some(t) = sim.0.target(f.0) else { continue };
        if matches!(t.state, TargetState::Dead) {
            *vis = Visibility::Hidden;
            continue;
        }
        *vis = Visibility::Inherited;
        *shape = match (&anims.0, &map.0) {
            (Some(set), Some(m)) if t.ai.is_some() => {
                let Some(a) = t.enemy_anim() else { continue };
                if pose::enemy_clip(set, a.style, a.slot).is_none()
                    && warned.insert(format!("{:?}/{}", a.style, a.slot))
                {
                    warn!(
                        "enemy clip not found for slot {} (style {:?}); tried {:?}; playing STAND",
                        a.slot,
                        a.style,
                        crate::enemy::clip_candidates(a.style, a.slot)
                    );
                }
                let def = &sim.0.weapons[t.ai.as_ref().map_or(0, |ai| ai.weapon.def)];
                let goal = a.aim.map(|p| pose::AimGoal {
                    target: (Quat::from_rotation_y(-t.yaw) * (bv(p) - bv(t.pos))).to_array(),
                    weight: 1.0,
                });
                let joints = pose::enemy_joints(set, m, &a, def, goal);
                shape_from_joints(m, &joints, bv(t.pos), t.yaw, Some(def), &mut unused)
            }
            (Some(set), Some(m)) => {
                let (clip, time) = t.anim();
                let joints = pose::clip_joints(set, &clip, time);
                shape_from_joints(m, &joints, bv(t.pos), t.yaw, None, &mut unused)
            }
            _ => {
                let pts = rest_points(&sim.0, bv(t.pos), t.yaw);
                shape_from_skeleton(
                    &skeleton_parents(&sim.0),
                    sim.0.skeleton.joint("Bip01 Head"),
                    &pts,
                    1.0,
                )
            }
        };
    }
}

pub fn pose_corpses(sim: Res<PayneSim>, mut q: Query<(&CorpseFigure, &mut Shape)>) {
    for (f, mut shape) in &mut q {
        let Some(c) = sim.0.corpses.get(f.0) else {
            continue;
        };
        let pts: Vec<Vec3> = c.ragdoll.positions().into_iter().map(bv).collect();
        *shape = shape_from_skeleton(
            &skeleton_parents(&sim.0),
            sim.0.skeleton.joint("Bip01 Head"),
            &pts,
            0.9,
        );
    }
}

/// Write the pooled child transforms for every figure's shape.
pub fn draw_figures(
    mut commands: Commands,
    time: Res<Time<Real>>,
    assets: Res<RigAssets>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut figs: Query<(Entity, &mut Figure, &Shape)>,
    mut parts: Query<(&mut Transform, &mut Visibility), Without<Figure>>,
) {
    for (root, mut f, shape) in &mut figs {
        f.flash = (f.flash - time.delta_secs() * 5.0).max(0.0);
        if let Some(mut m) = mats.get_mut(&f.mat) {
            let e = f.flash;
            m.emissive = LinearRgba::rgb(e * 0.9, e * 0.55, e * 0.25);
        }
        let segs: Vec<Transform> = shape
            .segs
            .iter()
            .filter(|(a, b, _)| a.distance(*b) > 1e-4)
            .map(|&(a, b, r)| {
                let d = b - a;
                let len = d.length();
                Transform {
                    translation: (a + b) * 0.5,
                    rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
                    scale: Vec3::new(r, len, r),
                }
            })
            .collect();
        let balls: Vec<Transform> = shape
            .balls
            .iter()
            .map(|&(p, r)| Transform::from_translation(p).with_scale(Vec3::splat(r)))
            .collect();
        let boxes: Vec<Transform> = shape
            .boxes
            .iter()
            .map(|&(p, q, s)| Transform {
                translation: p,
                rotation: q,
                scale: s,
            })
            .collect();
        let gun_mat = assets.gun.clone();
        let fm = f.mat.clone();
        let f = &mut *f;
        pool(
            &mut commands,
            root,
            &mut parts,
            &mut f.cyl,
            &segs,
            &assets.cyl,
            &fm,
        );
        pool(
            &mut commands,
            root,
            &mut parts,
            &mut f.ball,
            &balls,
            &assets.ball,
            &fm,
        );
        pool(
            &mut commands,
            root,
            &mut parts,
            &mut f.cube,
            &boxes,
            &assets.cube,
            &gun_mat,
        );
    }
}

fn pool(
    commands: &mut Commands,
    root: Entity,
    parts: &mut Query<(&mut Transform, &mut Visibility), Without<Figure>>,
    pool: &mut Vec<Entity>,
    want: &[Transform],
    mesh: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
) {
    for (i, t) in want.iter().enumerate() {
        if let Some(&e) = pool.get(i)
            && let Ok((mut tf, mut v)) = parts.get_mut(e)
        {
            *tf = *t;
            *v = Visibility::Inherited;
        } else if i >= pool.len() {
            let e = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    *t,
                    ChildOf(root),
                ))
                .id();
            pool.push(e);
        }
    }
    for &e in pool.iter().skip(want.len()) {
        if let Ok((_, mut v)) = parts.get_mut(e) {
            *v = Visibility::Hidden;
        }
    }
}

/// Thin bright streaks for projectiles. They are drawn at least `MIN_STREAK` long so they stay
/// visible when bullet time shortens each step.
const MIN_STREAK: f32 = 1.4;
/// Enemy bullets fly at 50 m/s (a metre per step), so they get a longer, fatter streak. ESTIMATE:
/// by eye, so they read in bullet time and across the range.
const MIN_STREAK_ENEMY: f32 = 1.8;

pub fn draw_streaks(
    mut commands: Commands,
    sim: Res<PayneSim>,
    cam: Res<CameraRig>,
    assets: Res<RigAssets>,
    mut streaks: ResMut<Streaks>,
    mut parts: Query<
        (
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<Figure>,
    >,
) {
    let eye = cam.position();
    for (i, p) in sim.0.projectiles.iter().enumerate() {
        let pos = bv(p.pos);
        let dir = bv(p.dir);
        let enemy = p.owner.is_some();
        let len =
            bv(p.pos - p.prev)
                .length()
                .max(if enemy { MIN_STREAK_ENEMY } else { MIN_STREAK });
        // A bit thicker with distance so far shots stay visible.
        let w = (0.03 + 0.006 * pos.distance(eye)) * if enemy { 1.3 } else { 1.0 };
        let mat = if enemy {
            &assets.enemy_streak
        } else {
            &assets.streak
        };
        let t = Transform::from_translation(pos - dir * (len * 0.5))
            .looking_to(dir, Vec3::Y)
            .with_scale(Vec3::new(w, w, len));
        if let Some(&e) = streaks.0.get(i)
            && let Ok((mut tf, mut v, mut m)) = parts.get_mut(e)
        {
            *tf = t;
            *v = Visibility::Inherited;
            if m.0 != *mat {
                m.0 = mat.clone();
            }
        } else if i >= streaks.0.len() {
            let e = commands
                .spawn((Mesh3d(assets.cube.clone()), MeshMaterial3d(mat.clone()), t))
                .id();
            streaks.0.push(e);
        }
    }
    for &e in streaks.0.iter().skip(sim.0.projectiles.len()) {
        if let Ok((_, mut v, _)) = parts.get_mut(e) {
            *v = Visibility::Hidden;
        }
    }
}

/// Muzzle flashes, sparks and hit flashes from the sim's events.
pub fn handle_events(
    mut commands: Commands,
    mut events: MessageReader<PayneEvent>,
    assets: Res<RigAssets>,
    guns: Res<GunPoints>,
    mut figs: Query<(&TargetFigure, &mut Figure)>,
) {
    use crate::event::Event::*;
    let flash = |commands: &mut Commands,
                 at: Vec3,
                 r: f32,
                 life: f32,
                 light: bool,
                 mat: &Handle<StandardMaterial>| {
        let mut e = commands.spawn((
            Mesh3d(assets.ball.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(at).with_scale(Vec3::splat(r)),
            Fx(life),
        ));
        if light {
            e.with_child((
                PointLight {
                    intensity: 150_000.0,
                    range: 8.0,
                    color: Color::srgb(1.0, 0.8, 0.5),
                    ..default()
                },
                Transform::default(),
            ));
        }
    };
    for PayneEvent(ev) in events.read() {
        match ev {
            Shot { origin, hand, .. } => {
                let at = match hand {
                    Hand::Right => guns.right,
                    Hand::Left => guns.left,
                }
                .unwrap_or(bv(*origin));
                flash(&mut commands, at, 0.1, 0.05, true, &assets.flash);
            }
            Hit {
                target,
                point,
                zone,
                damage,
                ..
            } => {
                debug!("hit {target:?} {zone:?} for {damage:.0}");
                flash(&mut commands, bv(*point), 0.06, 0.15, false, &assets.spark);
                if let Some(id) = target {
                    for (f, mut fig) in &mut figs {
                        if f.0 == *id {
                            fig.flash = 1.0;
                        }
                    }
                }
            }
            EnemyShot { origin, .. } => {
                flash(&mut commands, bv(*origin), 0.12, 0.06, true, &assets.flash);
            }
            BodyHit { point, .. } => {
                flash(&mut commands, bv(*point), 0.05, 0.12, false, &assets.spark)
            }
            Kill { target, zone } => {
                info!("kill target {} ({zone:?})", target.0);
                for (f, mut fig) in &mut figs {
                    if f.0 == *target {
                        fig.flash = 1.5;
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn fx_tick(mut commands: Commands, time: Res<Time<Real>>, mut q: Query<(Entity, &mut Fx)>) {
    for (e, mut fx) in &mut q {
        fx.0 -= time.delta_secs();
        if fx.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// F1: collider, aim ray and point, hit zones.
pub fn debug_draw(ui: Res<PayneUi>, sim: Res<PayneSim>, guns: Res<GunPoints>, mut gizmos: Gizmos) {
    if !ui.debug {
        return;
    }
    let (s, p) = (&sim.0, &sim.0.player);
    let tu = &s.tuning;
    let top = match p.state {
        crate::State::Crouch | crate::State::Dodge { .. } => tu.crouch_top,
        crate::State::Prone { .. }
        | crate::State::GetUp { .. }
        | crate::State::Shootdodge { .. }
        | crate::State::Dead { .. } => tu.prone_top,
        _ => tu.capsule_top,
    };
    let col = Color::srgb(0.2, 1.0, 0.4);
    capsule(&mut gizmos, bv(p.pos), top, tu.capsule_radius, col);
    let chest = bv(p.chest(tu));
    gizmos.line(chest, bv(p.aim_target), Color::srgb(1.0, 0.2, 0.2));
    gizmos.sphere(
        Isometry3d::from_translation(bv(p.aim_target)),
        0.12,
        Color::srgb(1.0, 0.2, 0.2),
    );
    gizmos.ray(chest, bv(p.aim_dir(tu)) * 3.0, Color::srgb(1.0, 0.9, 0.2));
    // Each barrel, drawn as far as the aim point: its end should sit on the red ball.
    let reach = |o: Vec3| o.distance(bv(p.aim_target));
    for &(o, d) in &guns.barrels {
        gizmos.line(o, o + d * reach(o), Color::srgb(0.1, 1.0, 1.0));
    }
    for t in s.targets.iter().filter(|t| t.alive()) {
        if let Some(ai) = &t.ai {
            enemy_debug(&mut gizmos, t, ai, bv(p.chest(tu)));
        }
        for z in &t.def.zones {
            let c = match z.zone {
                crate::HitZone::Head => Color::srgb(1.0, 0.3, 0.3),
                crate::HitZone::Torso => Color::srgb(1.0, 0.8, 0.2),
                _ => Color::srgb(0.3, 0.7, 1.0),
            };
            let (a, b) = (bv(t.to_world(z.a)), bv(t.to_world(z.b)));
            gizmos.sphere(Isometry3d::from_translation(a), z.radius, c);
            if a.distance(b) > 1e-3 {
                gizmos.sphere(Isometry3d::from_translation(b), z.radius, c);
                gizmos.line(a, b, c);
            }
        }
    }
}

/// Sight line (green while it sees Max, dim red otherwise), a mode-coloured marker over the head
/// and the cover point it is running to.
fn enemy_debug(g: &mut Gizmos, t: &crate::Target, ai: &crate::enemy::Ai, player_chest: Vec3) {
    use crate::enemy::Mode;
    let eye = bv(t.pos) + Vec3::Y * (1.6 * t.stance);
    let (line, mark) = if ai.sees {
        (Color::srgb(0.2, 1.0, 0.3), Color::srgb(1.0, 0.2, 0.2))
    } else {
        (
            Color::srgba(0.8, 0.2, 0.2, 0.35),
            Color::srgb(0.7, 0.7, 0.7),
        )
    };
    g.line(eye, player_chest, line);
    let mark = match ai.mode {
        Mode::Idle => mark,
        Mode::Alerted { .. } => Color::srgb(1.0, 0.9, 0.2),
        Mode::Combat => Color::srgb(1.0, 0.2, 0.2),
        Mode::TakeCover { .. } => Color::srgb(0.3, 0.6, 1.0),
        Mode::InCover { .. } => Color::srgb(0.2, 0.4, 1.0),
        Mode::Search { .. } => Color::srgb(1.0, 0.6, 0.1),
    };
    g.sphere(
        Isometry3d::from_translation(bv(t.pos) + Vec3::Y * (2.1 * t.stance)),
        0.1,
        mark,
    );
    if let Mode::TakeCover { to, .. } | Mode::Search { to, .. } = ai.mode {
        g.line(
            bv(t.pos) + Vec3::Y * 0.1,
            bv(to) + Vec3::Y * 0.1,
            Color::srgb(0.3, 0.6, 1.0),
        );
        g.sphere(
            Isometry3d::from_translation(bv(to) + Vec3::Y * 0.1),
            0.15,
            Color::srgb(0.3, 0.6, 1.0),
        );
    }
}

fn capsule(g: &mut Gizmos, feet: Vec3, top: f32, r: f32, c: Color) {
    let (lo, hi) = (feet + Vec3::Y * r, feet + Vec3::Y * (top - r).max(r));
    g.sphere(Isometry3d::from_translation(lo), r, c);
    g.sphere(Isometry3d::from_translation(hi), r, c);
    for d in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
        g.line(lo + d * r, hi + d * r, c);
    }
}

/// The test world's boxes as cubes.
pub fn spawn_boxes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    boxes: &[Aabb],
) {
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    for (i, b) in boxes.iter().enumerate() {
        let shade = 0.38 + 0.06 * (i % 4) as f32;
        let size = bv(b.max) - bv(b.min);
        commands.spawn((
            Mesh3d(cube.clone()),
            MeshMaterial3d(mats.add(StandardMaterial {
                base_color: Color::srgb(shade, shade * 0.95, shade * 0.85),
                perceptual_roughness: 0.9,
                ..default()
            })),
            Transform::from_translation((bv(b.min) + bv(b.max)) * 0.5).with_scale(size),
        ));
    }
}

/// Ground plane, sun and ambient light.
pub fn spawn_ground(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        MeshMaterial3d(mats.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.22, 0.22),
            perceptual_roughness: 1.0,
            ..default()
        })),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(20.0, 50.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.85, 0.9, 1.0),
        brightness: 600.0,
        ..default()
    });
}
