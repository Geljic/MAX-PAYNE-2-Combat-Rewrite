//! `payne-extract`: reads the player's own Max Payne 2 install and writes the
//! runtime data `payne` loads (`assets/generated/`). Nothing it writes is
//! committed.
//!
//! Subcommands: `dump <file>` (implemented), `setup` and `ras` (planned).

mod ras;

use std::path::Path;
use std::process::ExitCode;

use payne_extract::kf2::{self, Kf2File};
use payne_extract::skd::Skeleton;
use payne_extract::xform;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dump") => match args.get(1) {
            Some(path) => match dump(path) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("payne-extract: {path}: {e}");
                    ExitCode::FAILURE
                }
            },
            None => usage("dump needs a file"),
        },
        Some("ras") => match (args.get(1), args.get(2)) {
            (Some(a), Some(o)) => match ras::extract(Path::new(a), Path::new(o)) {
                Ok(n) => {
                    println!("{a}: wrote {n} files to {o}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("payne-extract: {a}: {e}");
                    ExitCode::FAILURE
                }
            },
            _ => usage("ras needs <archive.ras> <out_dir>"),
        },
        Some("unpack") => match (args.get(1), args.get(2)) {
            (Some(g), Some(o)) => unpack(Path::new(g), Path::new(o)),
            _ => usage("unpack needs <game_dir> <out_dir>"),
        },
        Some("bake") => match (args.get(1), args.get(2)) {
            (Some(g), Some(o)) => match payne_extract::bake::bake(Path::new(g), Path::new(o)) {
                Ok(s) => {
                    for n in &s.notes {
                        eprintln!("note: {n}");
                    }
                    println!("baked {} clips, {} bones, {} bytes", s.clips, s.bones, s.bytes);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("payne-extract: bake: {e}");
                    ExitCode::FAILURE
                }
            },
            _ => usage("bake needs <mp2-files dir> <out anims.bin>"),
        },
        Some("generate") => match (args.get(1), args.get(2)) {
            (Some(m), Some(o)) => match payne_extract::generate::generate(Path::new(m), Path::new(o)) {
                Ok(r) => {
                    println!("{}", r.summary());
                    for w in &r.warnings {
                        println!("warning: {w}");
                    }
                    if r.errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
                }
                Err(e) => {
                    eprintln!("generate: {e}");
                    ExitCode::FAILURE
                }
            },
            _ => usage("generate needs <mp2-files dir> <out payne.data>"),
        },
        Some("measure") => match (args.get(1), args.get(2)) {
            (Some(d), Some(a)) => match measure(Path::new(d), Path::new(a)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("payne-extract: measure: {e}");
                    ExitCode::FAILURE
                }
            },
            _ => usage("measure needs <payne.data> <anims.bin>"),
        },
        Some("setup") => {
            eprintln!("payne-extract: `{}` is not implemented yet", args[0]);
            ExitCode::FAILURE
        }
        _ => usage("no command given"),
    }
}

/// Extract every `MP2_*.ras` in the game directory into `<out>/<archive stem>/`.
fn unpack(game: &Path, out: &Path) -> ExitCode {
    let Ok(rd) = std::fs::read_dir(game) else {
        eprintln!("payne-extract: cannot read {}", game.display());
        return ExitCode::FAILURE;
    };
    let mut archives: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            n.starts_with("mp2_") && n.ends_with(".ras")
        })
        .collect();
    archives.sort();
    let mut ok = true;
    for a in &archives {
        let stem = a.file_stem().and_then(|s| s.to_str()).unwrap_or("archive");
        match ras::extract(a, &out.join(stem)) {
            Ok(n) => println!("{stem}: {n} files"),
            Err(e) => {
                eprintln!("payne-extract: {}: {e}", a.display());
                ok = false;
            }
        }
    }
    if archives.is_empty() {
        eprintln!("payne-extract: no MP2_*.ras in {}", game.display());
        ok = false;
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Measure the baked animations and write the results into `payne.data` (idempotent).
fn measure(data_path: &Path, anims_path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(data_path).map_err(|e| format!("{}: {e}", data_path.display()))?;
    let bytes = std::fs::read(anims_path).map_err(|e| format!("{}: {e}", anims_path.display()))?;
    let set = payne::clips::AnimSet::load(&bytes).map_err(|e| e.to_string())?;
    let data = payne::DataFile::parse(&text).map_err(|e| e.to_string())?;
    let m = payne_extract::measure::measure(&set, &data);
    std::fs::write(data_path, payne_extract::measure::apply(&text, &m))
        .map_err(|e| format!("{}: {e}", data_path.display()))?;
    print!("{}", payne_extract::measure::summary(&m));
    println!("wrote {} character values and {} anim sections to {}", m.character.len(), m.anims.len(), data_path.display());
    Ok(())
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("payne-extract: {msg}\n\nusage:\n  payne-extract dump <file.kf2|.kfs|.skd>   print a summary of the file\n  payne-extract ras <archive.ras> <out_dir>   extract one archive\n  payne-extract unpack <game_dir> <out_dir>   extract every MP2_*.ras\n  payne-extract bake <mp2-files dir> <out>    bake skeleton + clips into anims.bin\n  payne-extract measure <payne.data> <anims.bin>   write measured tuning values into payne.data");
    ExitCode::from(2)
}

fn dump(path: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let file = kf2::parse(&data).map_err(|e| e.to_string())?;
    let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
    print!("{}", summarize(&file, stem, data.len()));
    Ok(())
}

fn summarize(f: &Kf2File, name: &str, size: usize) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "{name}: {size} bytes; {} meshes, {} skins, {} tracks, {} materials lists, {} cameras",
        f.meshes.len(),
        f.skins.len(),
        f.tracks.len(),
        f.material_lists.len(),
        f.cameras.len()
    );

    let sk = f.skeleton();
    if !sk.bones.is_empty() {
        let _ = writeln!(o, "\nbones ({}):", sk.bones.len());
        let mut seen = vec![false; sk.bones.len()];
        for root in sk.roots() {
            bone_tree(&sk, root, 1, &mut seen, &mut o);
        }
        for (i, s) in seen.iter().enumerate() {
            if !s {
                let _ = writeln!(o, "  (in a parent cycle) {}", sk.bones[i].name);
            }
        }
    }

    if !f.meshes.is_empty() {
        let _ = writeln!(o, "\nmeshes:");
        for m in &f.meshes {
            if m.positions.is_empty() && m.reference.is_none() {
                continue;
            }
            let _ = writeln!(
                o,
                "  {:<24} v{} verts {:>5} tris {:>5} materials {:?} uv layers {}{}",
                m.name(),
                m.version,
                m.positions.len(),
                m.triangles.len(),
                m.material_names,
                m.uv_layers.len(),
                m.reference.as_ref().map(|r| format!(" ref '{r}'")).unwrap_or_default()
            );
        }
    }

    for ml in &f.material_lists {
        let _ = writeln!(o, "\nmaterials (texture dirs '{}'):", ml.texture_dirs);
        for m in &ml.materials {
            let tex = m.diffuse_texture.as_ref().map(|t| t.files.join(",")).unwrap_or_default();
            let _ = writeln!(o, "  {:<24} diffuse '{tex}'{}", m.name, if m.two_sided { " two-sided" } else { "" });
        }
    }

    for s in &f.skins {
        let _ = writeln!(
            o,
            "\nskin v{}: {} skinned vertices, bones {:?}, meshes {:?}",
            s.version,
            s.vertices.len(),
            s.skeleton_objects,
            s.skin_objects
        );
        let max_inf = s.vertices.iter().map(|v| v.bones.len()).max().unwrap_or(0);
        let _ = writeln!(o, "  max influences per vertex: {max_inf}");
    }

    if let Some(c) = f.clip(name) {
        let _ = writeln!(
            o,
            "\nclip '{}': {} fps, {} frames, {:.3} s{}",
            c.name,
            c.fps,
            c.frame_count,
            c.duration_secs,
            if c.looping { ", looping" } else { "" }
        );
        for t in &c.tracks {
            let moves = t
                .keys
                .windows(2)
                .any(|w| xform::translation(&w[0].transform) != xform::translation(&w[1].transform));
            let _ = writeln!(
                o,
                "  {:<24} v{} parent '{}' keys {:>4} vis {:>3}{}",
                t.object_name,
                t.version,
                t.parent_name,
                t.keys.len(),
                t.visibility.len(),
                if moves { " (translates)" } else { "" }
            );
        }
    }

    for cam in &f.cameras {
        let _ = writeln!(o, "\ncamera '{}': fov {} near {} far {}", cam.node.name, cam.fov, cam.near, cam.far);
    }
    if !f.notes.is_empty() {
        let _ = writeln!(o, "\nnotes:");
        for n in &f.notes {
            let _ = writeln!(o, "  {n}");
        }
    }
    o
}

fn bone_tree(sk: &Skeleton, i: usize, depth: usize, seen: &mut [bool], o: &mut String) {
    use std::fmt::Write;
    if seen[i] {
        return;
    }
    seen[i] = true;
    let b = &sk.bones[i];
    let t = xform::translation(&b.bind_local);
    let _ = writeln!(o, "{}{}  t=({:.2}, {:.2}, {:.2})", "  ".repeat(depth), b.name, t[0], t[1], t[2]);
    for c in sk.children(i) {
        bone_tree(sk, c, depth + 1, seen, o);
    }
}
