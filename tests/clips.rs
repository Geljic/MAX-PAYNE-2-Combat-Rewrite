//! Clip loader and sampler: synthetic unit tests plus, when the generated
//! `assets/generated/anims.bin` exists, checks against the real baked data.

use payne::clips::{AnimSet, quat_rotate};

struct B(Vec<u8>);

impl B {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn f(&mut self, v: &[f32]) {
        for x in v {
            self.0.extend(x.to_le_bytes());
        }
    }
    fn s(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.0.extend(s.as_bytes());
    }
}

/// Two bones (root, arm 1 m along +X), a looping 4-frame clip that rotates the
/// arm a quarter turn about Z, and a one-shot with root motion.
fn tiny() -> Vec<u8> {
    let mut b = B(Vec::new());
    b.0.extend(b"PAYNEANM");
    b.u32(2);
    b.u32(2);
    b.s("root");
    b.0.extend((-1i32).to_le_bytes());
    b.f(&[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    b.s("arm");
    b.0.extend(0i32.to_le_bytes());
    b.f(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    b.u32(2);

    b.s("SPIN");
    b.s("spin.kf2");
    b.f(&[4.0]);
    b.u32(4);
    b.u8(1);
    b.u32(1);
    b.u16(1);
    b.u8(0); // translation constant
    b.f(&[1.0, 0.0, 0.0]);
    for f in 0..4 {
        // Half-angle about Z of f * 22.5 degrees.
        let a = (f as f32) * std::f32::consts::FRAC_PI_8;
        for q in [0.0, 0.0, a.sin(), a.cos()] {
            b.0.extend(((q * 32767.0f32).round() as i16).to_le_bytes());
        }
    }
    b.u8(0);

    b.s("WALK");
    b.s("walk.kf2");
    b.f(&[10.0]);
    b.u32(2);
    b.u8(0);
    b.u32(0);
    b.u8(1);
    b.f(&[10.0]);
    b.u32(11);
    for f in 0..11 {
        b.f(&[0.0, 0.0, f as f32 * 0.5, f as f32 * 0.1]);
    }
    b.0
}

#[test]
fn loads_and_looks_up() {
    let set = AnimSet::load(&tiny()).unwrap();
    assert_eq!(set.bones.len(), 2);
    assert_eq!(set.clips.len(), 2);
    assert!(set.clip("spin").is_some());
    assert!(set.clip("CHARANIM_WALK").is_some());
    assert!(set.clip("nope").is_none());
}

#[test]
fn rejects_garbage() {
    let good = tiny();
    assert!(AnimSet::load(b"nonsense").is_err());
    for cut in [8, 12, 40, good.len() - 1] {
        assert!(AnimSet::load(&good[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn sampling_interpolates_and_wraps() {
    let set = AnimSet::load(&tiny()).unwrap();
    let spin = set.clip("SPIN").unwrap();
    assert!((spin.duration() - 1.0).abs() < 1e-6, "looping period is frame_count / fps");
    let mut pose = set.bind_pose();
    // Halfway between frame 0 and 1: x translation still 1, rotation half of frame 1.
    spin.sample(&set, 0.125, &mut pose);
    let half = std::f32::consts::FRAC_PI_8 * 0.5;
    assert!((pose.q[1][2] - half.sin()).abs() < 1e-3, "{:?}", pose.q[1]);
    // t = 1.0 wraps to frame 0.
    spin.sample(&set, 1.0, &mut pose);
    assert!(pose.q[1][2].abs() < 1e-5);
    // Between the last frame and frame 0 of a loop it blends back, not clamps.
    spin.sample(&set, 0.875, &mut pose);
    let last = (3.0 * std::f32::consts::FRAC_PI_8).sin();
    assert!(pose.q[1][2] > 0.0 && pose.q[1][2] < last);
    // Unanimated root keeps the bind pose.
    assert_eq!(pose.t[0], [0.0, 1.0, 0.0]);
}

#[test]
fn model_space_composes_parent_first() {
    let set = AnimSet::load(&tiny()).unwrap();
    let mut pose = set.bind_pose();
    // Turn the root a quarter turn about Y: the arm (1 m along +X) points to -Z.
    let s = std::f32::consts::FRAC_1_SQRT_2;
    pose.q[0] = [0.0, s, 0.0, s];
    let j = set.model_space(&pose);
    assert!((j[1].pos[0]).abs() < 1e-5 && (j[1].pos[2] + 1.0).abs() < 1e-5 && (j[1].pos[1] - 1.0).abs() < 1e-5);
    let v = quat_rotate(j[1].rot, [1.0, 0.0, 0.0]);
    assert!((v[2] + 1.0).abs() < 1e-5);
}

#[test]
fn root_motion_deltas() {
    let set = AnimSet::load(&tiny()).unwrap();
    let walk = set.clip("WALK").unwrap();
    assert!((walk.duration() - 0.1).abs() < 1e-6);
    let (d, yaw) = walk.root_motion(0.0, 1.0);
    assert!((d[2] - 5.0).abs() < 1e-4 && (yaw - 1.0).abs() < 1e-4);
    let (d, _) = walk.root_motion(0.2, 0.5);
    assert!((d[2] - 1.5).abs() < 1e-4);
    // Clamped after the end for a one-shot.
    let (d, _) = walk.root_motion(1.0, 2.0);
    assert!(d[2].abs() < 1e-6);
    assert_eq!(set.clip("SPIN").unwrap().root_motion(0.0, 1.0), ([0.0; 3], 0.0));
}

// ---- real data ----

fn real() -> Option<AnimSet> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/generated/anims.bin");
    let bytes = std::fs::read(p).ok()?;
    Some(AnimSet::load(&bytes).expect("real anims.bin loads"))
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Bones the animators move relative to their parent on purpose: the hip
/// joints (thighs hang off the spine and are shifted as it bends), shoulder,
/// jacket, back and gun attachments, and the floor helper. Everything else is
/// a limb or spine segment and must keep its bind length.
fn authored_slack(name: &str) -> bool {
    ["Thigh", "Shoulder", "Jacket", "Back", "Gun", "Tongue", "Lip", "Cheek", "Eye", "Blink", "Jaw", "Brow", "Finger", "Belt", "Tail", "Clavicle"].iter().any(|k| name.contains(k))
}

#[test]
fn real_bone_lengths_stay_rigid() {
    let Some(set) = real() else { return };
    let bind = set.model_space(&set.bind_pose());
    let mut worst = (0.0f32, String::new());
    let mut worst_slack = (0.0f32, String::new());
    let mut pose = set.bind_pose();
    for c in &set.clips {
        for f in 0..c.frame_count {
            c.sample(&set, f as f32 / c.fps, &mut pose);
            let m = set.model_space(&pose);
            for (i, b) in set.bones.iter().enumerate() {
                let Some(p) = b.parent else { continue };
                let l0 = dist(bind[i].pos, bind[p].pos);
                let abs = (dist(m[i].pos, m[p].pos) - l0).abs();
                let at = format!("{} frame {f} bone {}", c.name, b.name);
                if b.name.contains("Footsteps") {
                    continue; // floor helper: moves relative to the root by design
                } else if authored_slack(&b.name) {
                    if abs > worst_slack.0 {
                        worst_slack = (abs, at);
                    }
                } else if l0 >= 0.02 && abs / l0 > worst.0 {
                    // Face bones (< 2 cm) are skipped: they are animated and tiny.
                    worst = (abs / l0, at);
                }
            }
        }
    }
    println!("worst limb/spine bone length deviation {:.3}% at {}", worst.0 * 100.0, worst.1);
    println!("worst authored-slack bone moves {:.1} cm at {}", worst_slack.0 * 100.0, worst_slack.1);
    assert!(worst.0 < 0.01, "{worst:?}");
    assert!(worst_slack.0 < 0.10, "{worst_slack:?}");
}

#[test]
fn real_standing_clips_touch_the_floor() {
    let Some(set) = real() else { return };
    let mut pose = set.bind_pose();
    for name in ["POSE", "STAND", "RUNFORWARD", "RUNBACKWARD"] {
        let Some(c) = set.clip(name) else { continue };
        let mut lowest = f32::MAX;
        let mut head = (f32::MAX, f32::MIN);
        for f in 0..c.frame_count {
            c.sample(&set, f as f32 / c.fps, &mut pose);
            let m = set.model_space(&pose);
            for (i, b) in set.bones.iter().enumerate() {
                if b.name.contains("Foot") && !b.name.contains("Footsteps") || b.name.contains("Toe") {
                    lowest = lowest.min(m[i].pos[1]);
                }
            }
            let h = m[set.bone_index("Bip01 Head").unwrap()].pos[1];
            head = (head.0.min(h), head.1.max(h));
        }
        println!("{name}: lowest foot/toe y {lowest:.3}, head y {:.3}..{:.3}", head.0, head.1);
        assert!(lowest.abs() < 0.05, "{name}: lowest {lowest}");
        assert!(head.0 > 1.4 && head.1 < 1.95, "{name}: head {head:?}");
    }
}

#[test]
fn real_root_motion_totals() {
    let Some(set) = real() else { return };
    let d = set.clip("SHOOTDODGEFORWARD").unwrap();
    let (m, _) = d.root_motion(0.0, 100.0);
    println!("SHOOTDODGEFORWARD root motion {m:?}");
    assert!((m[2] - 5.80).abs() < 0.05, "{m:?}");
    let r = set.clip("RUNFORWARD").unwrap();
    let (m, _) = r.root_motion(0.0, r.duration());
    println!("RUNFORWARD root motion over one cycle {m:?}, duration {}", r.duration());
    assert!((m[2] - 4.20).abs() < 0.05, "{m:?}");
}
