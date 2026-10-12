//! Baked animation clips: loader, sampler and forward kinematics.
//!
//! `payne-extract bake` turns the game's keyframe files into one dense,
//! little-endian `anims.bin` (layout in `docs/formats/anims-bin.md`). This
//! module reads that file with no dependencies and no engine types, so any
//! host (or the headless tests) can pose the skeleton.
//!
//! Conventions (all inferred and checked against the real data, see the doc):
//! Y up, metres, +Z is the character's forward, quaternions are `[x, y, z, w]`
//! and rotate column vectors (`v' = q v q*`). Joint transforms compose
//! `world = parent_world * local`.

use std::fmt;

/// Position in metres.
pub type Vec3 = [f32; 3];
/// Unit quaternion `[x, y, z, w]`.
pub type Quat = [f32; 4];

const MAGIC: &[u8; 8] = b"PAYNEANM";
const VERSION: u32 = 2;
const IDENT: Quat = [0.0, 0.0, 0.0, 1.0];

/// Why a file failed to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError(pub String);

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "anims.bin: {}", self.0)
    }
}

impl std::error::Error for LoadError {}

/// One skeleton joint with its bind-pose local transform.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Index of the parent bone, always lower than this bone's index.
    pub parent: Option<usize>,
    pub bind_t: Vec3,
    pub bind_q: Quat,
}

/// Root motion of a clip: where the movement dummy goes, per frame, relative
/// to its start. Hosts that keep the character in place simply ignore it.
#[derive(Debug, Clone, PartialEq)]
pub struct RootTrack {
    pub fps: f32,
    /// `(x, y, z, yaw)` per frame; frame 0 is all zero. Yaw is in radians,
    /// positive turning from +Z towards +X, unwrapped (may exceed pi).
    pub frames: Vec<[f32; 4]>,
}

/// One baked animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// Game animation name without the `CHARANIM_` prefix, e.g. `SHOOTDODGEFORWARD`.
    pub name: String,
    pub source: String,
    pub fps: f32,
    pub frame_count: usize,
    pub looping: bool,
    /// Skeleton indices of the bones this clip animates.
    pub bones: Vec<u16>,
    /// Rotations as normalised i16 quaternions, frame-major: frame `f`, track `i`
    /// is entry `f * bones.len() + i`.
    rot: Vec<[i16; 4]>,
    /// Per track: constant translation, used when `t_slot` is `None`.
    t_const: Vec<Vec3>,
    /// Per track: which animated-translation column it owns, if any.
    t_slot: Vec<Option<usize>>,
    /// Animated translations, frame-major over the animated columns.
    t_frames: Vec<Vec3>,
    t_cols: usize,
    pub root: Option<RootTrack>,
}

/// Local (parent-relative) transform of every skeleton bone.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub t: Vec<Vec3>,
    pub q: Vec<Quat>,
}

/// World-space (model-space) transform of a joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Joint {
    pub pos: Vec3,
    pub rot: Quat,
}

/// The skeleton plus every baked clip.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimSet {
    pub bones: Vec<Bone>,
    pub clips: Vec<Clip>,
}

// ---- tiny math ----

fn dot(a: Quat, b: Quat) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn normalize(q: Quat) -> Quat {
    let n = dot(q, q).sqrt();
    if n < 1e-9 {
        IDENT
    } else {
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    }
}

/// Hamilton product: `a * b` applies `b` first, then `a`.
pub fn quat_mul(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

/// Rotate a vector by a unit quaternion.
pub fn quat_rotate(q: Quat, v: Vec3) -> Vec3 {
    let u = [q[0], q[1], q[2]];
    let c1 = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let t = [2.0 * c1[0], 2.0 * c1[1], 2.0 * c1[2]];
    let c2 = [
        u[1] * t[2] - u[2] * t[1],
        u[2] * t[0] - u[0] * t[2],
        u[0] * t[1] - u[1] * t[0],
    ];
    [
        v[0] + q[3] * t[0] + c2[0],
        v[1] + q[3] * t[1] + c2[1],
        v[2] + q[3] * t[2] + c2[2],
    ]
}

/// Shortest-path spherical interpolation.
pub fn quat_slerp(a: Quat, mut b: Quat, t: f32) -> Quat {
    let mut d = dot(a, b);
    if d < 0.0 {
        d = -d;
        b = [-b[0], -b[1], -b[2], -b[3]];
    }
    if d > 0.9995 {
        return normalize([
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
            a[3] + (b[3] - a[3]) * t,
        ]);
    }
    let th = d.clamp(-1.0, 1.0).acos();
    let s = th.sin();
    let (wa, wb) = (((1.0 - t) * th).sin() / s, (t * th).sin() / s);
    [
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
        a[3] * wa + b[3] * wb,
    ]
}

fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

// ---- reading ----

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], LoadError> {
        let end = self.p.checked_add(n).filter(|e| *e <= self.b.len());
        let end = end.ok_or_else(|| LoadError(format!("truncated at byte {}", self.p)))?;
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, LoadError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, LoadError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, LoadError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, LoadError> {
        Ok(self.u32()? as i32)
    }
    fn f32(&mut self) -> Result<f32, LoadError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Result<String, LoadError> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| LoadError("name is not UTF-8".into()))
    }
    fn vec3(&mut self) -> Result<Vec3, LoadError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    fn quat(&mut self) -> Result<Quat, LoadError> {
        Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
    }
    /// A count that must fit in what is left of the file at `min_each` bytes per item.
    fn count(&mut self, min_each: usize) -> Result<usize, LoadError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min_each) > self.b.len() - self.p {
            return Err(LoadError(format!(
                "count {n} at byte {} exceeds the file",
                self.p
            )));
        }
        Ok(n)
    }
}

impl AnimSet {
    /// Parse the bytes of an `anims.bin`.
    pub fn load(bytes: &[u8]) -> Result<AnimSet, LoadError> {
        let mut r = Rd { b: bytes, p: 0 };
        if r.take(8)? != MAGIC {
            return Err(LoadError("bad magic".into()));
        }
        let v = r.u32()?;
        if v != VERSION {
            return Err(LoadError(format!("unsupported version {v}")));
        }
        let nb = r.count(2 + 4 + 28)?;
        let mut bones = Vec::with_capacity(nb);
        for i in 0..nb {
            let name = r.str()?;
            let p = r.i32()?;
            let parent = if p < 0 { None } else { Some(p as usize) };
            if parent.is_some_and(|p| p >= i) {
                return Err(LoadError(format!(
                    "bone {i} has a parent that does not come before it"
                )));
            }
            bones.push(Bone {
                name,
                parent,
                bind_t: r.vec3()?,
                bind_q: r.quat()?,
            });
        }
        let nc = r.count(10)?;
        let mut clips = Vec::with_capacity(nc);
        for _ in 0..nc {
            let name = r.str()?;
            let source = r.str()?;
            let fps = r.f32()?;
            let frame_count = r.u32()? as usize;
            let looping = r.u8()? != 0;
            let na = r.count(2)?;
            let mut idx = Vec::with_capacity(na);
            for _ in 0..na {
                let b = r.u16()?;
                if b as usize >= nb {
                    return Err(LoadError(format!(
                        "clip {name}: bone index {b} out of range"
                    )));
                }
                idx.push(b);
            }
            let mut animated = Vec::with_capacity(na);
            for _ in 0..na {
                animated.push(r.u8()? != 0);
            }
            let mut t_const = vec![[0.0; 3]; na];
            let mut t_slot = vec![None; na];
            let mut t_cols = 0;
            for k in 0..na {
                if animated[k] {
                    t_slot[k] = Some(t_cols);
                    t_cols += 1;
                } else {
                    t_const[k] = r.vec3()?;
                }
            }
            let nrot = frame_count
                .checked_mul(na)
                .filter(|t| t.saturating_mul(8) <= bytes.len() - r.p)
                .ok_or_else(|| LoadError(format!("clip {name}: sample data exceeds the file")))?;
            let mut rot = Vec::with_capacity(nrot);
            for _ in 0..nrot {
                rot.push([
                    r.u16()? as i16,
                    r.u16()? as i16,
                    r.u16()? as i16,
                    r.u16()? as i16,
                ]);
            }
            let nt = frame_count
                .checked_mul(t_cols)
                .filter(|t| t.saturating_mul(12) <= bytes.len() - r.p)
                .ok_or_else(|| {
                    LoadError(format!("clip {name}: translation data exceeds the file"))
                })?;
            let mut t_frames = Vec::with_capacity(nt);
            for _ in 0..nt {
                t_frames.push(r.vec3()?);
            }
            let root = if r.u8()? != 0 {
                let rfps = r.f32()?;
                let n = r.count(16)?;
                let mut frames = Vec::with_capacity(n);
                for _ in 0..n {
                    frames.push([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
                }
                Some(RootTrack { fps: rfps, frames })
            } else {
                None
            };
            clips.push(Clip {
                name,
                source,
                fps,
                frame_count,
                looping,
                bones: idx,
                rot,
                t_const,
                t_slot,
                t_frames,
                t_cols,
                root,
            });
        }
        Ok(AnimSet { bones, clips })
    }

    /// Clip by game name (case-insensitive, with or without `CHARANIM_`).
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        let n = name.strip_prefix("CHARANIM_").unwrap_or(name);
        self.clips.iter().find(|c| c.name.eq_ignore_ascii_case(n))
    }

    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// The skeleton's bind pose.
    pub fn bind_pose(&self) -> Pose {
        Pose {
            t: self.bones.iter().map(|b| b.bind_t).collect(),
            q: self.bones.iter().map(|b| b.bind_q).collect(),
        }
    }

    /// Joint transforms in model space for `pose`.
    pub fn model_space(&self, pose: &Pose) -> Vec<Joint> {
        pose.model_space(&self.bones)
    }
}

impl Pose {
    /// World-space joints, composing `world = parent_world * local`.
    /// `bones` must be the skeleton the pose was made for.
    pub fn model_space(&self, bones: &[Bone]) -> Vec<Joint> {
        let mut out: Vec<Joint> = Vec::with_capacity(bones.len());
        for (i, b) in bones.iter().enumerate() {
            let j = match b.parent {
                Some(p) => {
                    let pj = out[p];
                    Joint {
                        pos: add(pj.pos, quat_rotate(pj.rot, self.t[i])),
                        rot: normalize(quat_mul(pj.rot, self.q[i])),
                    }
                }
                None => Joint {
                    pos: self.t[i],
                    rot: self.q[i],
                },
            };
            out.push(j);
        }
        out
    }
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

impl Clip {
    /// Length of one play-through in seconds. A looping clip's last frame
    /// blends back into frame 0, so its period is `frame_count / fps`; a
    /// one-shot ends on its last frame, at `(frame_count - 1) / fps`.
    pub fn duration(&self) -> f32 {
        if self.fps <= 0.0 {
            return 0.0;
        }
        let frames = if self.looping {
            self.frame_count
        } else {
            self.frame_count.saturating_sub(1)
        };
        frames as f32 / self.fps
    }

    /// Frame pair and blend factor for a time. Looping clips wrap, others clamp.
    fn locate(&self, t: f32) -> (usize, usize, f32) {
        let n = self.frame_count;
        if n <= 1 || self.fps <= 0.0 {
            return (0, 0, 0.0);
        }
        let mut f = t * self.fps;
        if self.looping {
            f = f.rem_euclid(n as f32);
            let a = (f.floor() as usize).min(n - 1);
            (a, (a + 1) % n, f - a as f32)
        } else {
            f = f.clamp(0.0, (n - 1) as f32);
            let a = (f.floor() as usize).min(n - 2);
            (a, a + 1, f - a as f32)
        }
    }

    fn quat_at(&self, i: usize) -> Quat {
        let q = self.rot[i];
        normalize([q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32])
    }

    /// Set the animated bones of `out` for time `t` (seconds), leaving the
    /// others as they are. Lets a host layer clips (e.g. aim over legs).
    pub fn sample_into(&self, t: f32, out: &mut Pose) {
        let (a, b, k) = self.locate(t);
        let w = self.bones.len();
        for (i, &bone) in self.bones.iter().enumerate() {
            let (ta, tb) = match self.t_slot[i] {
                Some(c) => (
                    self.t_frames[a * self.t_cols + c],
                    self.t_frames[b * self.t_cols + c],
                ),
                None => (self.t_const[i], self.t_const[i]),
            };
            let (qa, qb) = (self.quat_at(a * w + i), self.quat_at(b * w + i));
            out.t[bone as usize] = lerp3(ta, tb, k);
            out.q[bone as usize] = quat_slerp(qa, qb, k);
        }
    }

    /// Full pose at time `t`: bind pose for bones the clip does not animate.
    pub fn sample(&self, set: &AnimSet, t: f32, out: &mut Pose) {
        for (i, b) in set.bones.iter().enumerate() {
            out.t[i] = b.bind_t;
            out.q[i] = b.bind_q;
        }
        self.sample_into(t, out);
    }

    /// Root motion between two clip times: `(translation delta, yaw delta)`,
    /// in the character's frame at the start of the clip. Looping clips
    /// accumulate whole cycles, so `t1` may pass the duration. No root track
    /// means no motion.
    pub fn root_motion(&self, t0: f32, t1: f32) -> (Vec3, f32) {
        let Some(r) = &self.root else {
            return ([0.0; 3], 0.0);
        };
        let (p0, y0) = self.root_at(r, t0);
        let (p1, y1) = self.root_at(r, t1);
        ([p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]], y1 - y0)
    }

    /// Accumulated root offset at time `t`.
    fn root_at(&self, r: &RootTrack, t: f32) -> (Vec3, f32) {
        let n = r.frames.len();
        if n == 0 {
            return ([0.0; 3], 0.0);
        }
        let last = r.frames[n - 1];
        let dur = (n - 1) as f32 / r.fps.max(1e-6);
        let (cycles, tt) = if self.looping && dur > 0.0 {
            let c = (t / dur).floor();
            (c, t - c * dur)
        } else {
            (0.0, t.clamp(0.0, dur))
        };
        let f = (tt * r.fps).clamp(0.0, (n - 1) as f32);
        let a = (f.floor() as usize).min(n - 1);
        let b = (a + 1).min(n - 1);
        let k = f - a as f32;
        let (fa, fb) = (r.frames[a], r.frames[b]);
        let m = |i: usize| fa[i] + (fb[i] - fa[i]) * k + cycles * last[i];
        ([m(0), m(1), m(2)], m(3))
    }
}

impl Pose {
    /// Pose of a skeleton's bind pose (same as [`AnimSet::bind_pose`]).
    pub fn bind(bones: &[Bone]) -> Pose {
        Pose {
            t: bones.iter().map(|b| b.bind_t).collect(),
            q: bones.iter().map(|b| b.bind_q).collect(),
        }
    }
}
