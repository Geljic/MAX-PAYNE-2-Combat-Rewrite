//! Measures Max Payne 2's own animations and turns them into tuning values.
//!
//! `payne-extract measure <payne.data> <anims.bin>` reads the baked clips (`anims.bin`, see
//! `docs/formats/anims-bin.md`) and writes what it can measure into `payne.data`:
//!
//! - into the `[character MaxPayne]` section, as `Tuning` keys (run speeds, capsule tops,
//!   muzzle heights, landing, flinch and weapon-switch times);
//! - into the `[anim NAME]` sections of the timed moves (shootdodge, get-up, dodge, jump, spin),
//!   as `frames`, `fps` and `distance`;
//! - into every `[anim NAME]` section whose clip has a root track, the root-motion curve:
//!   `root = <d0> <d1> ... <dn>` (horizontal distance from the start at each clip frame, m) and
//!   `root_dir = <x> <z>` (overall horizontal travel direction in the clip's own frame).
//!
//! Everything is written between `# measured by payne-extract` and
//! `# end measured by payne-extract` comments, so running it again replaces the block instead of
//! adding a second one. What each number is and how it was derived is in
//! `docs/behaviour/measured.md`.

use std::collections::BTreeMap;
use std::fmt::Write;

use payne::clips::{AnimSet, Clip, Pose};
use payne::data::{DataFile, parse_number};

/// Opens a block this module wrote.
pub const BEGIN: &str = "# measured by payne-extract";
/// Closes it.
pub const END: &str = "# end measured by payne-extract";

/// Gap between a bone centre and the surface of the body, added where a capsule top is derived
/// from lying-down bone heights (m).
const BODY_MARGIN: f32 = 0.12;

/// The player's section in `payne.data`.
const PLAYER: &str = "MaxPayne";

/// Everything measured, ready to write.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Measured {
    /// `[character MaxPayne]` keys (Tuning field names) and values.
    pub character: Vec<(String, f32)>,
    /// `[anim NAME]` sections that get `frames`, `fps` and `distance` keys.
    pub anims: BTreeMap<String, Vec<(String, f32)>>,
    /// `[anim NAME]` keys whose value is a list (`root`, `root_dir`), already formatted.
    pub curves: BTreeMap<String, Vec<(String, String)>>,
    /// Evidence and things that could not be measured, one line each, for the console.
    pub notes: Vec<String>,
}

// ---------- helpers on clips ----------

/// Horizontal distance the root travels over the whole clip (m).
pub fn root_distance(c: &Clip) -> Option<f32> {
    let last = c.root.as_ref()?.frames.last()?;
    Some(last[0].hypot(last[2]))
}

/// Length of the root track in seconds.
pub fn root_duration(c: &Clip) -> Option<f32> {
    let r = c.root.as_ref()?;
    (r.frames.len() > 1 && r.fps > 0.0).then(|| (r.frames.len() - 1) as f32 / r.fps)
}

/// Horizontal root speed over the whole track (m/s). For a loop this is the speed of the cycle.
pub fn root_speed(c: &Clip) -> Option<f32> {
    Some(root_distance(c)? / root_duration(c)?)
}

/// Horizontal distance from the start (m) at each clip frame `0..frame_count`, sampled from the
/// root track at the clip's own frame times (the root track may run at a different rate). The
/// magnitude of the horizontal offset, so side and back moves give positive, growing curves.
pub fn root_curve(c: &Clip) -> Option<Vec<f32>> {
    c.root.as_ref()?;
    let fps = c.fps.max(1e-6);
    Some(
        (0..c.frame_count)
            .map(|f| {
                let (d, _) = c.root_motion(0.0, f as f32 / fps);
                d[0].hypot(d[2])
            })
            .collect(),
    )
}

/// Overall horizontal travel direction `(x, z)` in the clip's own frame, normalised: the last
/// root offset. `None` without a root track or when it ends where it began.
pub fn root_dir(c: &Clip) -> Option<[f32; 2]> {
    let last = c.root.as_ref()?.frames.last()?;
    let l = last[0].hypot(last[2]);
    (l > 1e-3).then(|| [last[0] / l, last[2] / l])
}

/// Model-space position of a bone at an integer frame.
pub fn bone_at(set: &AnimSet, c: &Clip, frame: usize, bone: &str) -> Option<[f32; 3]> {
    let i = set.bone_index(bone)?;
    let mut pose = Pose::bind(&set.bones);
    c.sample(set, frame as f32 / c.fps, &mut pose);
    Some(set.model_space(&pose)[i].pos)
}

/// Highest model-space `y` of any bone except the floor helper, at an integer frame.
pub fn top_at(set: &AnimSet, c: &Clip, frame: usize) -> f32 {
    let mut pose = Pose::bind(&set.bones);
    c.sample(set, frame as f32 / c.fps, &mut pose);
    let joints = set.model_space(&pose);
    set.bones
        .iter()
        .zip(&joints)
        .filter(|(b, _)| b.name != "Bip01 Footsteps")
        .map(|(_, j)| j.pos[1])
        .fold(f32::MIN, f32::max)
}

/// Frames where the pelvis is within `tol` of its lowest point: the held part of a pose that
/// first has to get there (the crouch clip starts standing).
pub fn settled_frames(set: &AnimSet, c: &Clip, tol: f32) -> Vec<usize> {
    let ys: Vec<f32> = (0..c.frame_count)
        .map(|f| bone_at(set, c, f, "Bip01").map_or(0.0, |p| p[1]))
        .collect();
    let low = ys.iter().copied().fold(f32::MAX, f32::min);
    (0..c.frame_count).filter(|&f| ys[f] <= low + tol).collect()
}

/// First frame after `takeoff` where the pelvis has come down to within `margin` of the lowest it
/// gets afterwards: the dive or roll has reached the floor.
pub fn landing_frame(set: &AnimSet, c: &Clip, takeoff: usize, margin: f32) -> Option<usize> {
    let ys: Vec<f32> = (takeoff..c.frame_count)
        .map(|f| bone_at(set, c, f, "Bip01").map(|p| p[1]))
        .collect::<Option<_>>()?;
    let low = ys.iter().copied().fold(f32::MAX, f32::min);
    ys.iter().position(|&y| y <= low + margin).map(|i| takeoff + i)
}

/// Gravity that makes a take-off at `v` m/s land after `frames` animation frames at `fps`.
pub fn implied_gravity(v: f32, frames: f32, fps: f32) -> f32 {
    2.0 * v * fps / frames
}

/// `C_Jump(v)` frame and argument on an anim section, if it has one.
fn jump_msg(data: &DataFile, anim: &str) -> Option<(f32, f32)> {
    let s = data.section("anim", anim)?;
    s.get_all("msg").find_map(|m| {
        let mut p = m.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
            .filter(|p| !p.is_empty());
        let frame = parse_number(p.next()?)?;
        let func = p.next()?.rsplit('.').next()?.to_ascii_lowercase();
        (func == "c_jump").then(|| Some((frame, parse_number(p.next()?)?)))?
    })
}

// ---------- the measurement ----------

/// Measure everything this module knows how to. `data` supplies message frames and the standing
/// capsule top; clips missing from `set` are skipped and noted.
pub fn measure(set: &AnimSet, data: &DataFile) -> Measured {
    let mut m = Measured::default();
    let put = |m: &mut Measured, k: &str, v: f32| m.character.push((k.to_string(), v));

    // Ground speeds from root travel per loop.
    let speed = |n: &str| set.clip(n).and_then(root_speed);
    if let Some(v) = speed("RUNFORWARD") {
        put(&mut m, "run_speed", v);
    }
    match (speed("RUNLEFT"), speed("RUNRIGHT")) {
        (Some(a), Some(b)) => put(&mut m, "strafe_speed", (a + b) / 2.0),
        _ => m.notes.push("strafe_speed: RUNLEFT/RUNRIGHT root track missing".into()),
    }
    if let Some(v) = speed("RUNBACKWARD") {
        put(&mut m, "backpedal_speed", v);
    }
    m.notes.push(
        "crouch_speed: not measurable, the retail data has no crouch-walk clip (CROUCH is a held pose)"
            .into(),
    );

    // Body heights.
    let capsule_top = data
        .section("character", PLAYER)
        .and_then(|s| s.num("capsule_top"))
        .unwrap_or(payne::Tuning::default().capsule_top);
    let stand_head = set
        .clip("STAND")
        .and_then(|c| bone_at(set, c, 0, "Bip01 Head"))
        .map(|p| p[1]);
    if let (Some(head), Some(crouch)) = (stand_head, set.clip("CROUCH")) {
        // The capsule reaches this far above the head bone when standing; keep that gap.
        let gap = capsule_top - head;
        let held = settled_frames(set, crouch, 0.05);
        let crouch_head = held
            .iter()
            .filter_map(|&f| bone_at(set, crouch, f, "Bip01 Head"))
            .map(|p| p[1])
            .fold(f32::MIN, f32::max);
        if !held.is_empty() {
            put(&mut m, "crouch_top", crouch_head + gap);
        }
    }

    // Muzzle: the gun bone of the aiming clip (hand height; the barrel adds a few cm forward, not up).
    if let Some(aim) = set.clip("SHOOTBERETTA") {
        let gun = bone_at(set, aim, 0, "Bip01 R Gun");
        let aim_pelvis = bone_at(set, aim, 0, "Bip01");
        if let (Some(g), Some(p)) = (gun, aim_pelvis) {
            put(&mut m, "muzzle_stand", g[1]);
            put(&mut m, "muzzle_side", g[0].abs());
            // Crouched, the upper body keeps its offset from the pelvis, which sits lower.
            if let Some(crouch) = set.clip("CROUCH") {
                let held = settled_frames(set, crouch, 0.05);
                let pelvis: Vec<f32> = held
                    .iter()
                    .filter_map(|&f| bone_at(set, crouch, f, "Bip01"))
                    .map(|p| p[1])
                    .collect();
                if !pelvis.is_empty() {
                    let mean = pelvis.iter().sum::<f32>() / pelvis.len() as f32;
                    put(&mut m, "muzzle_crouch", mean + (g[1] - p[1]));
                }
            }
        }
    }
    if let Some(dive) = set.clip("SHOOTDODGEFORWARD") {
        let last = dive.frame_count - 1;
        if let Some(g) = bone_at(set, dive, last, "Bip01 R Gun") {
            put(&mut m, "muzzle_prone", g[1]);
        }
    }

    // Lying: highest bone of the final pose of the four dives, plus the body's thickness.
    let tops: Vec<f32> = ["FORWARD", "BACKWARD", "LEFT", "RIGHT"]
        .iter()
        .filter_map(|d| set.clip(&format!("SHOOTDODGE{d}")))
        .map(|c| top_at(set, c, c.frame_count - 1))
        .collect();
    if !tops.is_empty() {
        let top = tops.iter().copied().fold(f32::MIN, f32::max);
        put(&mut m, "prone_top", top + BODY_MARGIN);
    }

    // Durations.
    if let Some(c) = set.clip("LAND") {
        // LAND carries the `dummy_land_hard` effect on its first frame.
        put(&mut m, "hard_land_time", c.duration());
        m.notes.push(
            "land_time: not measurable, the soft landing has no clip of its own (FALL only tags a soft-land effect)"
                .into(),
        );
    }
    if let Some(c) = set.clip("SMALLDAMAGE") {
        put(&mut m, "flinch_time", c.duration());
    }
    if let Some(c) = set.clip("GETWEAPON") {
        put(&mut m, "switch_time", c.duration());
    }

    // Timed moves: frames, fps and root distance, in the clip's own frames like the messages.
    let moves = [
        "SHOOTDODGEFORWARD",
        "SHOOTDODGEBACKWARD",
        "SHOOTDODGELEFT",
        "SHOOTDODGERIGHT",
        "SHOOTDODGEFORWARD_GETUP",
        "SHOOTDODGEBACKWARD_GETUP",
        "SHOOTDODGELEFT_GETUP",
        "SHOOTDODGERIGHT_GETUP",
        "DODGEFORWARD",
        "DODGEBACKWARD",
        "DODGELEFT",
        "DODGERIGHT",
        "JUMP",
        "JUMPFORWARD",
        "WHIRLLEFT",
    ];
    for name in moves {
        let Some(c) = set.clip(name) else {
            m.notes.push(format!("{name}: clip missing from anims.bin"));
            continue;
        };
        let mut keys = vec![
            ("frames".to_string(), c.frame_count as f32),
            ("fps".to_string(), c.fps),
        ];
        if let Some(d) = root_distance(c) {
            keys.push(("distance".to_string(), d));
        }
        m.anims.insert(name.to_string(), keys);
    }

    // Root-motion curves for every clip that moves (the sim follows them instead of a constant
    // speed). Only sections present in payne.data receive them.
    for c in &set.clips {
        let (Some(curve), Some(dir)) = (root_curve(c), root_dir(c)) else {
            continue;
        };
        let list = |v: &[f32]| v.iter().map(|x| fmt_value(*x)).collect::<Vec<_>>().join(" ");
        m.curves.insert(
            c.name.to_ascii_uppercase(),
            vec![
                ("root".to_string(), list(&curve)),
                ("root_dir".to_string(), list(&dir)),
            ],
        );
    }

    gravity_evidence(set, data, &mut m);
    m
}

/// Compare the take-off and landing frames of the moves that jump, and emit `gravity` only if
/// they all imply the same value.
fn gravity_evidence(set: &AnimSet, data: &DataFile, m: &mut Measured) {
    let mut implied = Vec::new();
    for name in [
        "SHOOTDODGEFORWARD",
        "SHOOTDODGEBACKWARD",
        "SHOOTDODGELEFT",
        "SHOOTDODGERIGHT",
        "DODGEFORWARD",
        "DODGELEFT",
        "DODGERIGHT",
        "DODGEBACKWARD",
    ] {
        let (Some(c), Some((frame, v))) = (set.clip(name), jump_msg(data, name).or_else(|| {
            // Directions without messages of their own play the forward dive's timeline.
            name.starts_with("SHOOTDODGE")
                .then(|| jump_msg(data, "SHOOTDODGEFORWARD"))
                .flatten()
        })) else {
            continue;
        };
        let takeoff = frame as usize;
        let mut line = format!("gravity evidence {name}: C_Jump({v}) at frame {frame}");
        let mut mid = None;
        for margin in [0.05, 0.10, 0.15] {
            // A body already on the floor at take-off (the back roll) shows no airborne phase.
            if let Some(l) = landing_frame(set, c, takeoff, margin).filter(|&l| l > takeoff) {
                let g = implied_gravity(v, (l - takeoff) as f32, c.fps);
                let _ = write!(line, "; margin {margin:.2} lands frame {l} => g {g:.1}");
                if margin == 0.10 {
                    mid = Some(g);
                }
            }
        }
        m.notes.push(line);
        implied.extend(mid);
    }
    if implied.is_empty() {
        return;
    }
    let mean = implied.iter().sum::<f32>() / implied.len() as f32;
    let agree = implied.iter().all(|g| (g - mean).abs() <= 0.15 * mean);
    if agree {
        m.character.push(("gravity".into(), mean));
        m.notes.push(format!("gravity: the moves agree, emitting {mean:.2}"));
    } else {
        let (lo, hi) = implied
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &g| (a.min(g), b.max(g)));
        m.notes.push(format!(
            "gravity: the moves disagree ({lo:.1} to {hi:.1} m/s2 implied), not emitted"
        ));
    }
    // The standing jump has no landing in its clip (the Fall loop takes over), so it can only be
    // compared by its apex.
    if let (Some(c), Some((frame, v))) = (set.clip("JUMP"), jump_msg(data, "JUMP")) {
        let air = (c.frame_count as f32 - frame) / c.fps;
        m.notes.push(format!(
            "JUMP: clip leaves {air:.2} s after C_Jump({v}); apex is {:.2} s away at 9.81 m/s2 ({:.2} m), so the clip ends before the apex and says nothing about gravity",
            v / 9.81,
            v * v / (2.0 * 9.81)
        ));
    }
}

// ---------- writing payne.data ----------

fn fmt_value(v: f32) -> String {
    // Four decimals: the measurements are good to a millimetre at best.
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn block(entries: &[(String, f32)]) -> Vec<String> {
    let text: Vec<(String, String)> =
        entries.iter().map(|(k, v)| (k.clone(), fmt_value(*v))).collect();
    text_block(&text)
}

fn text_block(entries: &[(String, String)]) -> Vec<String> {
    let mut b = vec![BEGIN.to_string()];
    b.extend(entries.iter().map(|(k, v)| format!("{k} = {v}")));
    b.push(END.to_string());
    b
}

/// Everything measured for one `[anim NAME]` section, numbers formatted.
fn anim_entries(m: &Measured, name: &str) -> Vec<(String, String)> {
    let mut e: Vec<(String, String)> = m
        .anims
        .get(name)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), fmt_value(*v)))
        .collect();
    e.extend(m.curves.get(name).into_iter().flatten().cloned());
    e
}

/// Remove every block this module wrote.
fn strip(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        match line.trim() {
            BEGIN => inside = true,
            END if inside => inside = false,
            _ if inside => {}
            _ => out.push(line),
        }
    }
    out
}

/// `payne.data` text with `m` written in: the old measured blocks are replaced, nothing else is
/// touched. The character block goes at the end of its section (later keys win there); anim
/// blocks go right after the header (the first key wins there).
pub fn apply(text: &str, m: &Measured) -> String {
    let lines = strip(text);
    let header = |l: &str| -> Option<(String, String)> {
        let h = l.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
        let (k, n) = h.split_once(char::is_whitespace).unwrap_or((h, ""));
        Some((k.to_ascii_lowercase(), n.trim().to_ascii_uppercase()))
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 200);
    let mut done_character = false;
    let mut done_anims: Vec<String> = Vec::new();
    // Index in `out` after the last non-blank line of the current character section.
    let mut char_end: Option<usize> = None;
    for l in &lines {
        if let Some((kind, name)) = header(l) {
            if let Some(i) = char_end.take() {
                out.splice(i..i, block(&m.character));
                done_character = true;
            }
            out.push(l.to_string());
            if kind == "character" && name == PLAYER.to_ascii_uppercase() && !done_character {
                char_end = Some(out.len());
            } else if kind == "anim" && !done_anims.contains(&name) {
                let entries = anim_entries(m, &name);
                if !entries.is_empty() {
                    out.extend(text_block(&entries));
                    done_anims.push(name);
                }
            }
            continue;
        }
        out.push(l.to_string());
        if char_end.is_some() && !l.trim().is_empty() {
            char_end = Some(out.len());
        }
    }
    if let Some(i) = char_end {
        out.splice(i..i, block(&m.character));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Console summary: each emitted value against what the defaults held.
pub fn summary(m: &Measured) -> String {
    let t = payne::Tuning::default();
    let mut s = String::new();
    for (k, v) in &m.character {
        let old = t.get(k).map_or("-".to_string(), fmt_value);
        let _ = writeln!(s, "{k:22} {:>9}  (default {old})", fmt_value(*v));
    }
    for (name, keys) in &m.anims {
        let kv: Vec<String> = keys.iter().map(|(k, v)| format!("{k} {}", fmt_value(*v))).collect();
        let _ = writeln!(s, "anim {name:26} {}", kv.join(", "));
    }
    for (name, keys) in &m.curves {
        if let Some((_, curve)) = keys.iter().find(|(k, _)| k == "root") {
            let n = curve.split_whitespace().count();
            let last = curve.split_whitespace().last().unwrap_or("0");
            let _ = writeln!(s, "root {name:26} {n} frames, ends at {last} m");
        }
    }
    for n in &m.notes {
        let _ = writeln!(s, "note: {n}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a tiny anims.bin: bones Bip01 > Bip01 Pelvis > Bip01 Head, plus a gun on the pelvis.
    /// Each clip is `(name, fps, looping, Bip01 y per frame, root frames)`.
    type Root = Option<(f32, Vec<[f32; 4]>)>;
    type Spec<'a> = (&'a str, f32, bool, Vec<f32>, Root);

    fn synthetic(clips: &[Spec]) -> AnimSet {
        let mut b: Vec<u8> = b"PAYNEANM".to_vec();
        let u32le = |b: &mut Vec<u8>, v: u32| b.extend(v.to_le_bytes());
        let f32le = |b: &mut Vec<u8>, v: f32| b.extend(v.to_le_bytes());
        let string = |b: &mut Vec<u8>, s: &str| {
            b.extend((s.len() as u16).to_le_bytes());
            b.extend(s.as_bytes());
        };
        u32le(&mut b, 2);
        let bones = [
            ("Bip01", -1i32, [0.0, 0.0, 0.0]),
            ("Bip01 Pelvis", 0, [0.0, 1.0, 0.0]),
            ("Bip01 Head", 1, [0.0, 0.6, 0.0]),
        ];
        u32le(&mut b, bones.len() as u32);
        for (n, p, t) in bones {
            string(&mut b, n);
            b.extend(p.to_le_bytes());
            for v in t {
                f32le(&mut b, v);
            }
            for v in [0.0, 0.0, 0.0, 1.0] {
                f32le(&mut b, v);
            }
        }
        u32le(&mut b, clips.len() as u32);
        for (name, fps, looping, pelvis_y, root) in clips {
            string(&mut b, name);
            string(&mut b, "synthetic.kf2");
            f32le(&mut b, *fps);
            u32le(&mut b, pelvis_y.len() as u32);
            b.push(*looping as u8);
            u32le(&mut b, 2);
            b.extend(0u16.to_le_bytes()); // Bip01 (the root bone, which carries height in the retail clips) animated
            b.extend(2u16.to_le_bytes()); // head constant
            b.extend([1u8, 0u8]);
            for v in [0.0, 0.6, 0.0] {
                f32le(&mut b, v);
            }
            for _ in pelvis_y {
                for _ in 0..2 {
                    for v in [0i16, 0, 0, 32767] {
                        b.extend(v.to_le_bytes());
                    }
                }
            }
            for y in pelvis_y {
                for v in [0.0, *y, 0.0] {
                    f32le(&mut b, v);
                }
            }
            match root {
                Some((rfps, frames)) => {
                    b.push(1);
                    f32le(&mut b, *rfps);
                    u32le(&mut b, frames.len() as u32);
                    for f in frames {
                        for v in f {
                            f32le(&mut b, *v);
                        }
                    }
                }
                None => b.push(0),
            }
        }
        AnimSet::load(&b).expect("synthetic anims.bin loads")
    }

    fn line_root(n: usize, step: f32) -> Root {
        Some((10.0, (0..n).map(|i| [0.0, 0.0, step * i as f32, 0.0]).collect()))
    }

    #[test]
    fn root_speed_is_distance_per_period() {
        // 41 root frames at 10 fps = 4 s, 0.5 m per frame = 20 m.
        let set = synthetic(&[("RUN", 10.0, true, vec![1.0; 40], line_root(41, 0.5))]);
        let c = set.clip("RUN").unwrap();
        assert!((root_distance(c).unwrap() - 20.0).abs() < 1e-4);
        assert!((root_duration(c).unwrap() - 4.0).abs() < 1e-5);
        assert!((root_speed(c).unwrap() - 5.0).abs() < 1e-4);
    }

    #[test]
    fn root_curve_is_distance_per_clip_frame() {
        // Root track at 10 fps going left 0.5 m per frame; the clip runs at 5 fps, so every clip
        // frame is two root frames.
        let root = Some((10.0, (0..9).map(|i| [-0.5 * i as f32, 0.0, 0.0, 0.0]).collect()));
        let set = synthetic(&[("DODGELEFT", 5.0, false, vec![1.0; 5], root)]);
        let c = set.clip("DODGELEFT").unwrap();
        let curve = root_curve(c).unwrap();
        assert_eq!(curve.len(), 5);
        for (f, d) in curve.iter().enumerate() {
            assert!((d - f as f32).abs() < 1e-4, "{curve:?}");
        }
        assert_eq!(root_dir(c), Some([-1.0, 0.0]));
        let m = measure(&set, &DataFile::default());
        assert_eq!(m.curves["DODGELEFT"][0], ("root".to_string(), "0 1 2 3 4".to_string()));
    }

    #[test]
    fn no_root_track_means_no_speed() {
        let set = synthetic(&[("STAND", 10.0, true, vec![1.0; 4], None)]);
        assert_eq!(root_speed(set.clip("STAND").unwrap()), None);
    }

    #[test]
    fn bone_heights_follow_the_pelvis_track() {
        let set = synthetic(&[("DIVE", 10.0, false, vec![1.0, 0.5, 0.2], None)]);
        let c = set.clip("DIVE").unwrap();
        let head = bone_at(&set, c, 1, "Bip01 Head").unwrap();
        assert!((head[1] - 2.1).abs() < 1e-4, "{head:?}");
        assert!((top_at(&set, c, 2) - 1.8).abs() < 1e-4);
    }

    #[test]
    fn settled_frames_skip_the_way_down() {
        let set = synthetic(&[("CROUCH", 10.0, true, vec![1.0, 0.7, 0.5, 0.51, 0.5], None)]);
        let c = set.clip("CROUCH").unwrap();
        assert_eq!(settled_frames(&set, c, 0.05), vec![2, 3, 4]);
    }

    #[test]
    fn landing_is_where_the_pelvis_reaches_the_floor() {
        // Rises after take-off at frame 1, comes down at frame 5 and stays.
        let ys = vec![0.6, 0.6, 0.9, 1.0, 0.7, 0.2, 0.15, 0.15];
        let set = synthetic(&[("DIVE", 10.0, false, ys, None)]);
        let c = set.clip("DIVE").unwrap();
        assert_eq!(landing_frame(&set, c, 1, 0.1), Some(5));
        // 5 m/s up, landing 0.4 s later: g = 2 v / t = 25.
        assert!((implied_gravity(5.0, 4.0, 10.0) - 25.0).abs() < 1e-5);
    }

    #[test]
    fn measure_emits_run_speeds_and_skips_what_is_missing() {
        let set = synthetic(&[
            ("RUNFORWARD", 10.0, true, vec![1.0; 40], line_root(41, 0.5)),
            ("RUNBACKWARD", 10.0, true, vec![1.0; 40], line_root(41, -0.25)),
        ]);
        let m = measure(&set, &DataFile::default());
        let get = |k: &str| m.character.iter().find(|(n, _)| n == k).map(|(_, v)| *v);
        assert!((get("run_speed").unwrap() - 5.0).abs() < 1e-4);
        assert!((get("backpedal_speed").unwrap() - 2.5).abs() < 1e-4);
        assert_eq!(get("strafe_speed"), None);
        assert!(m.notes.iter().any(|n| n.starts_with("strafe_speed")));
    }

    #[test]
    fn gravity_is_only_emitted_when_the_moves_agree() {
        let data = DataFile::parse(
            "[anim DODGELEFT]\nmsg = 1 this.C_Jump 5\n[anim DODGERIGHT]\nmsg = 1 this.C_Jump 5\n",
        )
        .unwrap();
        let ys = vec![0.6, 0.6, 0.9, 1.0, 0.7, 0.2, 0.15, 0.15];
        let same = synthetic(&[
            ("DODGELEFT", 10.0, false, ys.clone(), None),
            ("DODGERIGHT", 10.0, false, ys.clone(), None),
        ]);
        let m = measure(&same, &data);
        let g = m.character.iter().find(|(k, _)| k == "gravity").expect("agree").1;
        assert!((g - 25.0).abs() < 1e-3, "{g}");
        // The right dodge lands two frames later: 25 vs about 16.7.
        let late = vec![0.6, 0.6, 0.9, 1.0, 0.9, 0.8, 0.7, 0.15];
        let differ = synthetic(&[
            ("DODGELEFT", 10.0, false, ys, None),
            ("DODGERIGHT", 10.0, false, late, None),
        ]);
        let m = measure(&differ, &data);
        assert!(m.character.iter().all(|(k, _)| k != "gravity"));
        assert!(m.notes.iter().any(|n| n.contains("disagree")));
    }

    fn sample_measured() -> Measured {
        Measured {
            character: vec![("run_speed".into(), 5.25), ("strafe_speed".into(), 5.2)],
            anims: BTreeMap::from([(
                "DODGELEFT".to_string(),
                vec![("frames".to_string(), 31.0), ("distance".to_string(), 5.2431)],
            )]),
            curves: BTreeMap::from([(
                "DODGELEFT".to_string(),
                vec![
                    ("root".to_string(), "0 0.5 1.25".to_string()),
                    ("root_dir".to_string(), "-1 0".to_string()),
                ],
            )]),
            notes: vec![],
        }
    }

    const DATA: &str = "[character MaxPayne]\ncapsule_top = 1.95\n\n[character Other]\nmax_health = 1\n\n[anim DODGELEFT]\nid = 21\nfile = x.kf2\n\n[anim DODGERIGHT]\nid = 22\n";

    #[test]
    fn apply_reaches_style_prefixed_sections() {
        let mut m = sample_measured();
        m.curves.insert("SHARP/RUNFORWARD".to_string(), vec![("root".to_string(), "0 0.1".to_string()), ("root_dir".to_string(), "0 1".to_string())]);
        let data = "[anim SHARP/RUNFORWARD]\nid = 1\nfile = s.kf2\n\n[anim RUNFORWARD]\nid = 1\n";
        let file = DataFile::parse(&apply(data, &m)).expect("still parses");
        assert_eq!(file.section("anim", "SHARP/RUNFORWARD").unwrap().get("root"), Some("0 0.1"));
        assert_eq!(file.section("anim", "RUNFORWARD").unwrap().get("root"), None);
    }

    #[test]
    fn apply_writes_into_the_right_sections() {
        let out = apply(DATA, &sample_measured());
        let file = DataFile::parse(&out).expect("still parses");
        let c = file.section("character", "MaxPayne").unwrap();
        assert_eq!(c.num("capsule_top"), Some(1.95));
        assert_eq!(c.num("run_speed"), Some(5.25));
        assert_eq!(file.section("character", "Other").unwrap().num("run_speed"), None);
        let a = file.section("anim", "DODGELEFT").unwrap();
        assert_eq!(a.num("frames"), Some(31.0));
        assert_eq!(a.num("distance"), Some(5.2431));
        assert_eq!(a.get("file"), Some("x.kf2"));
        assert_eq!(a.get("root"), Some("0 0.5 1.25"));
        assert_eq!(a.get("root_dir"), Some("-1 0"));
        assert_eq!(file.section("anim", "DODGERIGHT").unwrap().num("frames"), None);
    }

    #[test]
    fn apply_twice_changes_nothing() {
        let once = apply(DATA, &sample_measured());
        let twice = apply(&once, &sample_measured());
        assert_eq!(once, twice);
        assert_eq!(once.matches(BEGIN).count(), 2);
        // A new measurement replaces the old one.
        let mut m = sample_measured();
        m.character[0].1 = 6.0;
        let third = apply(&once, &m);
        assert!(third.contains("run_speed = 6\n") && !third.contains("run_speed = 5.25"));
    }

    #[test]
    fn stripping_leaves_the_original_text() {
        let once = apply(DATA, &sample_measured());
        let bare = apply(&once, &Measured::default());
        // Empty blocks remain only where the character section is; the content is the original.
        let kept: Vec<&str> = bare.lines().filter(|l| !l.starts_with('#')).collect();
        let orig: Vec<&str> = DATA.lines().collect();
        assert_eq!(kept, orig);
    }
}
