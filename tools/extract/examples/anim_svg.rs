//! Stick-figure SVGs of baked clips, for eyeballing the rotation convention.
//! `cargo run -p payne-extract --example anim_svg -- <anims.bin> <out_dir> [CLIP...]`
//! Writes `<CLIP>_side.svg` (Z right, Y up) and `<CLIP>_front.svg` (X right, Y up),
//! six evenly spaced frames each. Left bones blue, right bones red, spine grey.

#[path = "../../../src/clips.rs"]
#[allow(dead_code)]
mod clips;

use clips::AnimSet;
use std::fmt::Write;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let set = AnimSet::load(&std::fs::read(&a[0]).expect("read anims.bin")).expect("load");
    std::fs::create_dir_all(&a[1]).unwrap();
    let names: Vec<String> = if a.len() > 2 { a[2..].to_vec() } else { vec!["SHOOTDODGEFORWARD".into(), "RUNFORWARD".into(), "STAND".into()] };
    for n in names {
        let Some(c) = set.clip(&n) else { eprintln!("no clip {n}"); continue };
        for (view, axis) in [("side", 2usize), ("front", 0usize)] {
            let svg = render(&set, c, axis);
            std::fs::write(format!("{}/{}_{view}.svg", a[1], c.name), svg).unwrap();
        }
    }
}

fn keep(name: &str) -> bool {
    !["Finger", "Eye", "Blink", "Brow", "Cheek", "Lip", "Jaw", "Tongue", "Jacket", "Back", "Belt", "Tail", "Gun", "Shoulder", "Footsteps"]
        .iter()
        .any(|k| name.contains(k))
}

fn render(set: &AnimSet, c: &clips::Clip, axis: usize) -> String {
    const N: usize = 6;
    let scale = 120.0; // px per metre
    let cell = 3.4 * scale;
    let (w, h) = (cell * N as f32, 2.4 * scale + 40.0);
    let mut o = String::new();
    let _ = writeln!(o, r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><rect width="100%" height="100%" fill="#fff"/>"##);
    let ground = h - 30.0;
    let _ = writeln!(o, r##"<line x1="0" y1="{ground}" x2="{w}" y2="{ground}" stroke="#999"/>"##);
    let _ = writeln!(o, r##"<text x="6" y="16" font-size="14" font-family="sans-serif">{} {} (axis {}; +{} to the right)</text>"##, c.name, if axis == 2 { "side" } else { "front" }, axis, if axis == 2 { "Z" } else { "X" });
    let mut pose = set.bind_pose();
    for k in 0..N {
        let f = k * (c.frame_count - 1) / (N - 1);
        c.sample(set, f as f32 / c.fps, &mut pose);
        let m = set.model_space(&pose);
        // Centre each figure on its pelvis along the view axis so travel does not push it off the cell.
        let pel = set.bone_index("Bip01 Pelvis").unwrap();
        let cx = cell * (k as f32 + 0.5);
        let px = |p: [f32; 3]| cx + (p[axis] - m[pel].pos[axis]) * scale;
        let py = |p: [f32; 3]| ground - p[1] * scale;
        let _ = writeln!(o, r##"<text x="{}" y="{}" font-size="11" font-family="sans-serif">f{f}</text>"##, cx - 10.0, h - 8.0);
        for (i, b) in set.bones.iter().enumerate() {
            let Some(p) = b.parent else { continue };
            if !keep(&b.name) || !keep(&set.bones[p].name) {
                continue;
            }
            let col = if b.name.contains(" L ") { "#1f5fd0" } else if b.name.contains(" R ") { "#d02020" } else { "#444" };
            let _ = writeln!(o, r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{col}" stroke-width="3" stroke-linecap="round"/>"##, px(m[p].pos), py(m[p].pos), px(m[i].pos), py(m[i].pos));
        }
        let head = set.bone_index("Bip01 Head").unwrap();
        let _ = writeln!(o, r##"<circle cx="{:.1}" cy="{:.1}" r="7" fill="#222"/>"##, px(m[head].pos), py(m[head].pos));
    }
    o.push_str("</svg>\n");
    o
}
