//! `payne-extract bake`: turns the player's skeleton and keyframe files into
//! one dense `anims.bin` that the core crate loads (`src/clips.rs`). Layout:
//! `docs/formats/anims-bin.md`.
//!
//! Which clips are baked comes from `payne.data` (`[anim NAME]` sections with
//! `file =` and optional `movement =`) when it sits next to the output, plus
//! every animation `skins/Male_Player.h` names. The two are merged by name.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::kf2::{self, Track};
use crate::xform;

const MAGIC: &[u8; 8] = b"PAYNEANM";
const VERSION: u32 = 2;
type Quat = [f32; 4];

/// One clip to bake: game name plus paths relative to `database/`.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub name: String,
    pub file: String,
    pub movement: Option<String>,
}

pub struct Summary {
    pub clips: usize,
    pub bones: usize,
    pub bytes: usize,
    pub notes: Vec<String>,
}

/// Find the `database` directory under an MP2 data dir.
fn database_dir(root: &Path) -> Option<PathBuf> {
    ["MP2_Data/data/database", "data/database", "database", "."]
        .iter()
        .map(|s| root.join(s))
        .find(|p| p.join("skeletons").is_dir())
}

/// Resolve a path with `\` or `/` separators case-insensitively under `base`.
fn find_ci(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for part in rel.split(['\\', '/']).filter(|p| !p.is_empty()) {
        let direct = cur.join(part);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let hit = fs::read_dir(&cur)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
        cur = hit.path();
    }
    Some(cur)
}

/// Pull `Key = "value"` / `Key = value;` out of a line, case-insensitive on the key.
fn field(line: &str, key: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let at = lower.find(&format!("{} =", key.to_ascii_lowercase()))? + key.len() + 2;
    let rest = line[at..].trim_start();
    let v = if let Some(r) = rest.strip_prefix('"') { r.split('"').next()? } else { rest.split([';', ' ', '\t']).next()? };
    Some(v.trim().to_string())
}

/// Clips named by `Male_Player.h` (paths relative to `skeletons/male/`).
pub fn jobs_from_header(text: &str) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        if line.contains("[Animation]") {
            if let (Some(idx), Some(file)) = (field(line, "Index"), field(line, "Filename")) {
                let name = idx.strip_prefix("CHARANIM_").unwrap_or(&idx).to_string();
                jobs.push(Job { name, file: format!("skeletons/male/{file}"), movement: None });
            }
        } else if line.contains("[Movement]")
            && let (Some(job), Some(file)) = (jobs.last_mut(), field(line, "Filename"))
        {
            job.movement = Some(format!("skeletons/male/{file}"));
        }
    }
    jobs
}

/// Clips named by `payne.data` `[anim NAME]` sections (paths relative to `database/`).
pub fn jobs_from_payne_data(text: &str) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(h) = line.strip_prefix("[anim ").and_then(|r| r.strip_suffix(']')) {
            jobs.push(Job { name: h.trim().to_string(), file: String::new(), movement: None });
        } else if line.starts_with('[') {
            jobs.push(Job { name: String::new(), file: String::new(), movement: None });
        } else if let Some(j) = jobs.last_mut()
            && let Some((k, v)) = line.split_once('=')
        {
            match k.trim() {
                "file" => j.file = v.trim().to_string(),
                "movement" => j.movement = Some(v.trim().to_string()).filter(|s| !s.is_empty()),
                _ => {}
            }
        }
    }
    jobs.retain(|j| !j.name.is_empty() && !j.file.is_empty());
    jobs
}

fn q_dot(a: Quat, b: Quat) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn slerp(a: Quat, mut b: Quat, t: f32) -> Quat {
    let mut d = q_dot(a, b);
    if d < 0.0 {
        d = -d;
        b = [-b[0], -b[1], -b[2], -b[3]];
    }
    let (wa, wb) = if d > 0.9995 {
        (1.0 - t, t)
    } else {
        let th = d.clamp(-1.0, 1.0).acos();
        let s = th.sin();
        (((1.0 - t) * th).sin() / s, (t * th).sin() / s)
    };
    let q = [a[0] * wa + b[0] * wb, a[1] * wa + b[1] * wb, a[2] * wa + b[2] * wb, a[3] * wa + b[3] * wb];
    let n = q_dot(q, q).sqrt().max(1e-9);
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

/// A track's transform at a (possibly fractional) frame: keys are lerped for
/// translation and slerped for rotation; before the first / after the last key it holds.
fn sample_track(keys: &[(u32, [f32; 3], Quat)], frame: f32) -> ([f32; 3], Quat) {
    let first = keys[0];
    if frame <= first.0 as f32 {
        return (first.1, first.2);
    }
    let last = keys[keys.len() - 1];
    if frame >= last.0 as f32 {
        return (last.1, last.2);
    }
    let i = keys.partition_point(|k| (k.0 as f32) <= frame) - 1;
    let (a, b) = (keys[i], keys[i + 1]);
    let span = (b.0 - a.0).max(1) as f32;
    let t = (frame - a.0 as f32) / span;
    let p = [a.1[0] + (b.1[0] - a.1[0]) * t, a.1[1] + (b.1[1] - a.1[1]) * t, a.1[2] + (b.1[2] - a.1[2]) * t];
    (p, slerp(a.2, b.2, t))
}

fn keys_of(t: &Track) -> Vec<(u32, [f32; 3], Quat)> {
    let mut v: Vec<_> = t
        .keys
        .iter()
        .map(|k| (k.frame, xform::translation(&k.transform), xform::rotation_quat(&k.transform)))
        .collect();
    v.sort_by_key(|k| k.0);
    v
}

/// Yaw (radians, +Z towards +X) of a rotation, read from where it sends +Z.
fn yaw_of(q: Quat) -> f32 {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let fx = 2.0 * (x * z + w * y);
    let fz = 1.0 - 2.0 * (x * x + y * y);
    fx.atan2(fz)
}

/// Per-frame `(x, y, z, yaw)` of a movement file, relative to its first frame, yaw unwrapped.
fn bake_root(track: &Track, frames: u32) -> Vec<[f32; 4]> {
    let keys = keys_of(track);
    let mut out = Vec::with_capacity(frames as usize);
    let (mut base, mut prev, mut turns) = (None, 0.0f32, 0.0f32);
    for f in 0..frames {
        let (p, q) = sample_track(&keys, f as f32);
        let mut yaw = yaw_of(q);
        if f > 0 {
            while yaw + turns - prev > std::f32::consts::PI {
                turns -= std::f32::consts::TAU;
            }
            while yaw + turns - prev < -std::f32::consts::PI {
                turns += std::f32::consts::TAU;
            }
        }
        yaw += turns;
        prev = yaw;
        let (p0, y0) = *base.get_or_insert((p, yaw));
        out.push([p[0] - p0[0], p[1] - p0[1], p[2] - p0[2], yaw - y0]);
    }
    out
}

fn q_mul(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn q_conj(q: Quat) -> Quat {
    [-q[0], -q[1], -q[2], q[3]]
}

fn q_rot(q: Quat, v: [f32; 3]) -> [f32; 3] {
    let p = q_mul(q_mul(q, [v[0], v[1], v[2], 0.0]), q_conj(q));
    [p[0], p[1], p[2]]
}

type Local = ([f32; 3], Quat);

/// Convert one frame of local transforms from the hierarchy an animation file
/// used (`file_parent`) to the skeleton's own. A handful of retail files hang
/// the thighs off the pelvis or a jacket off the clavicle where the skeleton
/// has them under the spine / shoulder; read with the skeleton's parents their
/// offsets would be wrong. Bones whose parent agrees are left untouched.
fn reparent(locals: &mut [Local], skel_parent: &[Option<usize>], file_parent: &[Option<usize>]) {
    fn world(i: usize, locals: &[Local], fp: &[Option<usize>], memo: &mut [Option<Local>]) -> Local {
        if let Some(w) = memo[i] {
            return w;
        }
        let w = match fp[i] {
            Some(p) => {
                let (pp, pq) = world(p, locals, fp, memo);
                let r = q_rot(pq, locals[i].0);
                ([pp[0] + r[0], pp[1] + r[1], pp[2] + r[2]], q_mul(pq, locals[i].1))
            }
            None => locals[i],
        };
        memo[i] = Some(w);
        w
    }
    let n = locals.len();
    let mut memo = vec![None; n];
    let worlds: Vec<Local> = (0..n).map(|i| world(i, locals, file_parent, &mut memo)).collect();
    for i in 0..n {
        if skel_parent[i] == file_parent[i] {
            continue;
        }
        locals[i] = match skel_parent[i] {
            Some(p) => {
                let (pp, pq) = worlds[p];
                let d = [worlds[i].0[0] - pp[0], worlds[i].0[1] - pp[1], worlds[i].0[2] - pp[2]];
                let ic = q_conj(pq);
                (q_rot(ic, d), q_mul(ic, worlds[i].1))
            }
            None => worlds[i],
        };
    }
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend(v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.0.extend(s.as_bytes());
    }
}

fn read_kf2(path: &Path) -> Result<kf2::Kf2File, String> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    kf2::parse(&data).map_err(|e| format!("{}: {e}", path.display()))
}

/// Bake everything under `mp2` (the `mp2-files` dir) into the file `out`
/// (an existing directory gets `anims.bin` inside it).
pub fn bake(mp2: &Path, out: &Path) -> Result<Summary, String> {
    let db = database_dir(mp2).ok_or_else(|| format!("no database/skeletons under {}", mp2.display()))?;
    let out_file = if out.is_dir() { out.join("anims.bin") } else { out.to_path_buf() };
    let mut notes = Vec::new();

    let skel_path = find_ci(&db, "skeletons/male/skeleton_male.kf2").ok_or("skeleton_male.kf2 not found")?;
    let skel_file = read_kf2(&skel_path)?;
    let skel = skel_file.skeleton();
    if skel.bones.is_empty() {
        return Err("skeleton_male.kf2 has no bones".into());
    }
    for (i, b) in skel.bones.iter().enumerate() {
        if b.parent.is_some_and(|p| p >= i) {
            return Err(format!("bone {} comes before its parent", b.name));
        }
    }

    let mut jobs: BTreeMap<String, Job> = BTreeMap::new();
    let data_path = out_file.parent().map(|d| d.join("payne.data")).filter(|p| p.is_file());
    if let Some(p) = data_path {
        let text = fs::read_to_string(&p).map_err(|e| e.to_string())?;
        for j in jobs_from_payne_data(&text) {
            jobs.insert(j.name.to_ascii_uppercase(), j);
        }
    }
    if let Some(h) = find_ci(&db, "skins/Male_Player.h") {
        let text = fs::read(&h).map_err(|e| e.to_string())?;
        for j in jobs_from_header(&String::from_utf8_lossy(&text)) {
            jobs.entry(j.name.to_ascii_uppercase()).or_insert(j);
        }
    } else {
        notes.push("skins/Male_Player.h not found".into());
    }
    // Idle clips the player header does not list but a host needs (standing
    // still, validation of the standing pose).
    for (n, f) in [("STAND", "skeletons/male/anim/Stand.kf2"), ("2_STAND", "skeletons/male/anim/2_Stand.kf2")] {
        jobs.entry(n.to_string()).or_insert(Job { name: n.to_string(), file: f.to_string(), movement: None });
    }
    if jobs.is_empty() {
        return Err("no clips to bake".into());
    }

    let mut w = W(Vec::new());
    w.0.extend(MAGIC);
    w.u32(VERSION);
    w.u32(skel.bones.len() as u32);
    for b in &skel.bones {
        w.str(&b.name);
        w.0.extend(b.parent.map(|p| p as i32).unwrap_or(-1).to_le_bytes());
        for v in xform::translation(&b.bind_local) {
            w.f32(v);
        }
        for v in xform::rotation_quat(&b.bind_local) {
            w.f32(v);
        }
    }

    let mut body = W(Vec::new());
    let mut count = 0u32;
    for (key, job) in &jobs {
        let name = key.as_str();
        let Some(path) = find_ci(&db, &job.file) else {
            notes.push(format!("{name}: {} not found, skipped", job.file));
            continue;
        };
        let file = match read_kf2(&path) {
            Ok(f) => f,
            Err(e) => {
                notes.push(format!("{name}: {e}, skipped"));
                continue;
            }
        };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
        let Some(clip) = file.clip(stem) else {
            notes.push(format!("{name}: no tracks, skipped"));
            continue;
        };
        let mut tracks: Vec<(usize, Vec<_>)> = Vec::new();
        for t in &clip.tracks {
            match skel.index_of(&t.object_name) {
                Some(i) if !t.keys.is_empty() => tracks.push((i, keys_of(t))),
                Some(_) => {}
                None => notes.push(format!("{name}: track '{}' is not a skeleton bone, dropped", t.object_name)),
            }
        }
        tracks.sort_by_key(|t| t.0);
        tracks.dedup_by_key(|t| t.0);

        body.str(name);
        body.str(path.file_name().and_then(|s| s.to_str()).unwrap_or(""));
        body.f32(clip.fps as f32);
        body.u32(clip.frame_count);
        body.u8(clip.looping as u8);
        body.u32(tracks.len() as u32);
        for (i, _) in &tracks {
            body.u16(*i as u16);
        }
        // Parent each track names, to catch files whose hierarchy differs from the skeleton's.
        let skel_parent: Vec<Option<usize>> = skel.bones.iter().map(|b| b.parent).collect();
        let mut file_parent = skel_parent.clone();
        for t in &clip.tracks {
            if let (Some(i), false) = (skel.index_of(&t.object_name), t.parent_name.is_empty())
                && let Some(p) = skel.index_of(&t.parent_name)
            {
                file_parent[i] = Some(p);
            }
        }
        let differs = file_parent != skel_parent;
        if differs {
            notes.push(format!("{name}: file hierarchy differs from skeleton, re-parented while baking"));
        }
        // Dense frames for every animated bone, already in the skeleton's hierarchy.
        let mut dense: Vec<Vec<Local>> = Vec::with_capacity(clip.frame_count as usize);
        for f in 0..clip.frame_count {
            let mut frame: Vec<Local> = skel
                .bones
                .iter()
                .map(|b| (xform::translation(&b.bind_local), xform::rotation_quat(&b.bind_local)))
                .collect();
            for (i, keys) in &tracks {
                frame[*i] = sample_track(keys, f as f32);
            }
            if differs {
                reparent(&mut frame, &skel_parent, &file_parent);
            }
            dense.push(tracks.iter().map(|(i, _)| frame[*i]).collect());
        }
        // A bone whose translation never changes stores it once.
        let animated_t: Vec<bool> = (0..tracks.len())
            .map(|k| {
                dense.iter().any(|fr| (0..3).any(|a| (fr[k].0[a] - dense[0][k].0[a]).abs() > 1e-5))
            })
            .collect();
        for &a in &animated_t {
            body.u8(a as u8);
        }
        for (k, &a) in animated_t.iter().enumerate() {
            if !a {
                for v in dense[0][k].0 {
                    body.f32(v);
                }
            }
        }
        for fr in &dense {
            for (_, q) in fr {
                for v in q {
                    body.0.extend(((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
                }
            }
        }
        for fr in &dense {
            for (k, (t, _)) in fr.iter().enumerate() {
                if animated_t[k] {
                    for v in t {
                        body.f32(*v);
                    }
                }
            }
        }

        let mov = job.movement.as_ref().and_then(|m| match find_ci(&db, m) {
            Some(p) => match read_kf2(&p) {
                Ok(f) => Some(f),
                Err(e) => {
                    notes.push(format!("{name}: movement {e}"));
                    None
                }
            },
            None => {
                notes.push(format!("{name}: movement {m} not found"));
                None
            }
        });
        match mov.as_ref().and_then(|f| f.clip("mov")) {
            Some(mc) => {
                let frames = bake_root(mc.tracks[0], mc.frame_count);
                body.u8(1);
                body.f32(mc.fps as f32);
                body.u32(frames.len() as u32);
                for fr in frames {
                    for v in fr {
                        body.f32(v);
                    }
                }
            }
            None => body.u8(0),
        }
        count += 1;
    }
    w.u32(count);
    w.0.extend(body.0);

    if let Some(dir) = out_file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&out_file, &w.0).map_err(|e| format!("{}: {e}", out_file.display()))?;
    Ok(Summary { clips: count as usize, bones: skel.bones.len(), bytes: w.0.len(), notes })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_prefixed_names_survive_the_data_file() {
        let t = "[anim SHARP/RUNFORWARD]\nfile = skeletons/male/anim/Sharp_Run.kf2\nmovement = skeletons/male/anim/Sharp_Run_mov.kf2\n\n[anim ENEMY/RUNFORWARD]\nfile = skeletons/male/anim/Run.kf2\n";
        let j = jobs_from_payne_data(t);
        assert_eq!(j.len(), 2);
        assert_eq!(j[0].name, "SHARP/RUNFORWARD");
        assert_eq!(j[0].movement.as_deref(), Some("skeletons/male/anim/Sharp_Run_mov.kf2"));
        assert_eq!(j[1].name, "ENEMY/RUNFORWARD");
        assert_eq!(j[1].movement, None);
    }

    #[test]
    fn header_jobs() {
        let t = "[Animation] Index = CHARANIM_RUNFORWARD; Filename = \"anim\\Run_Forward.kf2\";\n\t\t[Movement] Filename = \"anim\\Run_Forward_mov.kf2\";\n[Animation] Index = CHARANIM_POSE; Filename = \"pose_MaxPayne.kf2\";\n";
        let j = jobs_from_header(t);
        assert_eq!(j.len(), 2);
        assert_eq!(j[0].name, "RUNFORWARD");
        assert_eq!(j[0].file, "skeletons/male/anim\\Run_Forward.kf2");
        assert_eq!(j[0].movement.as_deref(), Some("skeletons/male/anim\\Run_Forward_mov.kf2"));
        assert_eq!(j[1].movement, None);
    }

    #[test]
    fn data_jobs() {
        let t = "[anim STAND]\nfile = skeletons/male/anim/Stand.kf2\nmovement = skeletons/male/anim/Stand_mov.kf2\n[other x]\nfile = no\n";
        let j = jobs_from_payne_data(t);
        assert_eq!(j.len(), 1);
        assert_eq!(j[0].name, "STAND");
        assert!(j[0].movement.is_some());
    }

    #[test]
    fn sparse_keys_interpolate() {
        let keys = vec![(0, [0.0; 3], [0.0, 0.0, 0.0, 1.0]), (10, [10.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0])];
        assert_eq!(sample_track(&keys, 5.0).0, [5.0, 0.0, 0.0]);
        assert_eq!(sample_track(&keys, 20.0).0, [10.0, 0.0, 0.0]);
    }
}
