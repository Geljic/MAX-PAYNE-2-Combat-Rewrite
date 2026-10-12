//! A small position-based ragdoll that needs no physics engine.
//!
//! Particles sit at the joints of a [`SkeletonDef`]. Each step: integrate velocities with gravity
//! and damping, then iterate distance constraints (bones and torso braces), angle limits (cones,
//! hinges) and collision against the [`World`], then rebuild velocities from the corrected
//! positions (Müller et al., "Position Based Dynamics"). Variable `dt` is fine, which matters
//! because bodies run on the slowed world clock in bullet time.
//!
//! The default [`SkeletonDef::humanoid`] (16 joints) uses MP2's ragdoll limits where they map
//! onto a particle model (`ragdolls/highdetail.txt`: knees 120°, elbows 130°, neck cone 20°,
//! spine 10°); positions, radii and masses are ESTIMATEs until the real male skeleton is loaded.
//! Self-collision between limbs is not modelled.

use crate::math::{Quat, Vec3};
use crate::tuning::Tuning;
use crate::world::World;

#[derive(Clone, Debug, PartialEq)]
pub struct JointDef {
    pub name: String,
    pub parent: Option<usize>,
    /// Rest position in the body frame: feet at the origin, facing -Z, +X to the right.
    pub rest: Vec3,
    /// Collision radius (m).
    pub radius: f32,
    /// Mass (kg). MP2 gives relative weights; we scale them to a 75 kg body.
    pub mass: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AngleLimit {
    /// The angle between directions a→b and c→d stays within `max` (rad); d is corrected.
    Cone {
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        max: f32,
    },
    /// Joint b bends only one way: c stays in the plane spanned by a→b and the body's forward,
    /// on the side given by `sign` (+1 bends forward like an elbow, -1 backward like a knee),
    /// within `max` (rad) of straight. Forward comes from `frame` = [left, right, up_from, up_to].
    Hinge {
        a: usize,
        b: usize,
        c: usize,
        max: f32,
        frame: [usize; 4],
        sign: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkeletonDef {
    pub joints: Vec<JointDef>,
    /// Extra distance constraints that keep the torso rigid.
    pub braces: Vec<(usize, usize)>,
    pub limits: Vec<AngleLimit>,
}

impl SkeletonDef {
    pub fn joint(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    /// Default 16-joint humanoid, arms hanging. Bone names follow the 3ds Max Biped names MP2 uses
    /// so data can map onto them.
    pub fn humanoid() -> SkeletonDef {
        let deg = |d: f32| d.to_radians();
        let mut joints = Vec::new();
        let mut add =
            |name: &str, parent: Option<usize>, rest: [f32; 3], radius: f32, mass: f32| {
                joints.push(JointDef {
                    name: name.to_string(),
                    parent,
                    rest: rest.into(),
                    radius,
                    mass,
                });
                joints.len() - 1
            };
        let pelvis = add("Bip01 Pelvis", None, [0.0, 0.95, 0.0], 0.14, 10.0);
        let spine = add("Bip01 Spine1", Some(pelvis), [0.0, 1.25, 0.0], 0.15, 12.0);
        let neck = add("Bip01 Neck", Some(spine), [0.0, 1.5, 0.0], 0.08, 4.0);
        let head = add("Bip01 Head", Some(neck), [0.0, 1.68, 0.0], 0.12, 5.0);
        let l_sh = add(
            "Bip01 L UpperArm",
            Some(spine),
            [-0.19, 1.45, 0.0],
            0.08,
            4.0,
        );
        let l_el = add("Bip01 L Forearm", Some(l_sh), [-0.22, 1.17, 0.0], 0.06, 2.5);
        let l_ha = add("Bip01 L Hand", Some(l_el), [-0.24, 0.92, 0.0], 0.05, 1.5);
        let r_sh = add(
            "Bip01 R UpperArm",
            Some(spine),
            [0.19, 1.45, 0.0],
            0.08,
            4.0,
        );
        let r_el = add("Bip01 R Forearm", Some(r_sh), [0.22, 1.17, 0.0], 0.06, 2.5);
        let r_ha = add("Bip01 R Hand", Some(r_el), [0.24, 0.92, 0.0], 0.05, 1.5);
        let l_hip = add("Bip01 L Thigh", Some(pelvis), [-0.1, 0.9, 0.0], 0.09, 8.0);
        let l_kn = add("Bip01 L Calf", Some(l_hip), [-0.11, 0.5, 0.0], 0.07, 4.0);
        let l_ft = add("Bip01 L Foot", Some(l_kn), [-0.11, 0.08, 0.0], 0.06, 2.0);
        let r_hip = add("Bip01 R Thigh", Some(pelvis), [0.1, 0.9, 0.0], 0.09, 8.0);
        let r_kn = add("Bip01 R Calf", Some(r_hip), [0.11, 0.5, 0.0], 0.07, 4.0);
        let r_ft = add("Bip01 R Foot", Some(r_kn), [0.11, 0.08, 0.0], 0.06, 2.0);
        let braces = vec![
            (l_sh, r_sh),
            (l_hip, r_hip),
            (l_sh, pelvis),
            (r_sh, pelvis),
            (l_hip, spine),
            (r_hip, spine),
            (l_sh, l_hip),
            (r_sh, r_hip),
            (neck, pelvis),
            (neck, l_sh),
            (neck, r_sh),
        ];
        let hips = [l_hip, r_hip, pelvis, spine];
        let shoulders = [l_sh, r_sh, spine, neck];
        let limits = vec![
            AngleLimit::Hinge {
                a: l_hip,
                b: l_kn,
                c: l_ft,
                max: deg(120.0),
                frame: hips,
                sign: -1.0,
            },
            AngleLimit::Hinge {
                a: r_hip,
                b: r_kn,
                c: r_ft,
                max: deg(120.0),
                frame: hips,
                sign: -1.0,
            },
            AngleLimit::Hinge {
                a: l_sh,
                b: l_el,
                c: l_ha,
                max: deg(130.0),
                frame: shoulders,
                sign: 1.0,
            },
            AngleLimit::Hinge {
                a: r_sh,
                b: r_el,
                c: r_ha,
                max: deg(130.0),
                frame: shoulders,
                sign: 1.0,
            },
            AngleLimit::Cone {
                a: spine,
                b: neck,
                c: neck,
                d: head,
                max: deg(20.0),
            },
            AngleLimit::Cone {
                a: pelvis,
                b: spine,
                c: spine,
                d: neck,
                max: deg(10.0),
            },
            // Thighs swing within 70° of the spine's downward line (MP2 leg ConeMax 70; we allow a
            // little more so a body can fold sitting).
            AngleLimit::Cone {
                a: spine,
                b: pelvis,
                c: l_hip,
                d: l_kn,
                max: deg(100.0),
            },
            AngleLimit::Cone {
                a: spine,
                b: pelvis,
                c: r_hip,
                d: r_kn,
                max: deg(100.0),
            },
        ];
        SkeletonDef {
            joints,
            braces,
            limits,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub inv_mass: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Link {
    a: usize,
    b: usize,
    rest: f32,
    bone: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ragdoll {
    pub particles: Vec<Particle>,
    links: Vec<Link>,
    limits: Vec<AngleLimit>,
    /// Rest directions of each bone (child − parent) in the body frame, for posing.
    rest_dirs: Vec<Option<Vec3>>,
    parents: Vec<Option<usize>>,
    /// The body's yaw when it was created (rest directions rotate with it).
    yaw: f32,
    asleep: bool,
    still_time: f32,
}

/// Fastest a particle may move (m/s): a guard against explosions.
const MAX_SPEED: f32 = 40.0;

impl Ragdoll {
    /// Build from joint positions in world space (e.g. the animated pose at death).
    pub fn from_pose(def: &SkeletonDef, joints: &[Vec3], yaw: f32, vel: Vec3) -> Ragdoll {
        let particles = def
            .joints
            .iter()
            .zip(joints)
            .map(|(j, &p)| Particle {
                pos: p,
                vel,
                inv_mass: if j.mass > 0.0 { 1.0 / j.mass } else { 0.0 },
                radius: j.radius,
            })
            .collect();
        let mut links = Vec::new();
        for (i, j) in def.joints.iter().enumerate() {
            if let Some(p) = j.parent {
                links.push(Link {
                    a: p,
                    b: i,
                    rest: joints[p].distance(joints[i]),
                    bone: true,
                });
            }
        }
        for &(a, b) in &def.braces {
            links.push(Link {
                a,
                b,
                rest: joints[a].distance(joints[b]),
                bone: false,
            });
        }
        let rest_dirs = def
            .joints
            .iter()
            .map(|j| {
                j.parent
                    .map(|p| (j.rest - def.joints[p].rest).normalize_or_zero())
            })
            .collect();
        Ragdoll {
            particles,
            links,
            limits: def.limits.clone(),
            rest_dirs,
            parents: def.joints.iter().map(|j| j.parent).collect(),
            yaw,
            asleep: false,
            still_time: 0.0,
        }
    }

    /// Build from the skeleton's rest pose placed at `feet` facing `yaw`.
    pub fn from_rest(def: &SkeletonDef, feet: Vec3, yaw: f32, vel: Vec3) -> Ragdoll {
        let q = Quat::from_yaw(yaw);
        let joints: Vec<Vec3> = def.joints.iter().map(|j| feet + q.rotate(j.rest)).collect();
        Ragdoll::from_pose(def, &joints, yaw, vel)
    }

    pub fn is_asleep(&self) -> bool {
        self.asleep
    }

    pub fn positions(&self) -> Vec<Vec3> {
        self.particles.iter().map(|p| p.pos).collect()
    }

    /// Largest relative bone length error, |len − rest| / rest, over all bones.
    pub fn bone_stretch(&self) -> f32 {
        self.links
            .iter()
            .filter(|l| l.bone)
            .map(|l| {
                ((self.particles[l.a].pos.distance(self.particles[l.b].pos) - l.rest) / l.rest)
                    .abs()
            })
            .fold(0.0, f32::max)
    }

    /// World rotation of the bone ending at `joint`: the shortest arc from its rest direction
    /// (turned by the body's yaw) to its current direction. No twist; good enough to pose limbs.
    pub fn bone_rotation(&self, joint: usize) -> Quat {
        match (self.parents[joint], self.rest_dirs[joint]) {
            (Some(p), Some(rest)) => {
                let cur = (self.particles[joint].pos - self.particles[p].pos).normalize_or_zero();
                Quat::from_rotation_arc(Quat::from_yaw(self.yaw).rotate(rest), cur)
            }
            _ => Quat::IDENTITY,
        }
    }

    /// Push the body at `point` with `impulse` (N·s), spread over the nearest particles.
    pub fn apply_impulse(&mut self, point: Vec3, impulse: Vec3) {
        let mut near: Vec<(usize, f32)> = self
            .particles
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.pos.distance(point)))
            .collect();
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        near.truncate(3);
        let weights: Vec<f32> = near.iter().map(|(_, d)| 1.0 / (d * d + 0.01)).collect();
        let total: f32 = weights.iter().sum();
        for ((i, _), w) in near.iter().zip(weights) {
            let p = &mut self.particles[*i];
            p.vel = (p.vel + impulse * (p.inv_mass * w / total)).clamp_length(MAX_SPEED);
        }
        self.asleep = false;
        self.still_time = 0.0;
    }

    /// Advance by `dt` world seconds.
    pub fn step<W: World + ?Sized>(&mut self, world: &W, dt: f32, tu: &Tuning) {
        if self.asleep || dt <= 0.0 {
            return;
        }
        let damp = (-tu.ragdoll_damping * dt).exp();
        let old: Vec<Vec3> = self.particles.iter().map(|p| p.pos).collect();
        let pre_vel: Vec<Vec3> = self.particles.iter().map(|p| p.vel).collect();
        for p in &mut self.particles {
            if p.inv_mass > 0.0 {
                p.vel = (p.vel + Vec3::new(0.0, -tu.gravity, 0.0) * dt) * damp;
                p.pos += p.vel * dt;
            }
        }

        let n = self.particles.len();
        let mut contact = vec![Vec3::ZERO; n]; // accumulated push-out per particle
        let iters = tu.ragdoll_iterations.max(1.0) as usize;
        for _ in 0..iters {
            for l in &self.links {
                solve_distance(&mut self.particles, l.a, l.b, l.rest);
            }
            for lim in &self.limits {
                solve_limit(&mut self.particles, lim);
            }
            for (i, p) in self.particles.iter_mut().enumerate() {
                let push = world.push_out(p.pos, p.pos, p.radius);
                if push.length_sq() > 0.0 {
                    p.pos += push;
                    contact[i] += push;
                }
            }
        }

        // Friction and restitution at contacts, then velocities from positions.
        let mut fastest = 0.0f32;
        for (i, p) in self.particles.iter_mut().enumerate() {
            let pen = contact[i].length();
            if pen > 0.0 {
                let nrm = contact[i] / pen;
                let delta = p.pos - old[i];
                let tangent = delta - nrm * delta.dot(nrm);
                let tl = tangent.length();
                // Coulomb-style position friction: static below mu·penetration, else kinetic.
                let mu = tu.ragdoll_friction;
                if tl <= mu * pen {
                    p.pos -= tangent;
                } else if tl > 0.0 {
                    p.pos -= tangent * (mu * pen / tl);
                }
            }
            p.vel = ((p.pos - old[i]) / dt).clamp_length(MAX_SPEED);
            if pen > 0.0 {
                let nrm = contact[i] / pen;
                let vn_in = pre_vel[i].dot(nrm);
                // Bounce only real impacts, so resting contacts stay at rest.
                if vn_in < -1.0 {
                    let vn = p.vel.dot(nrm);
                    p.vel += nrm * (-tu.ragdoll_restitution * vn_in - vn);
                }
            }
            if !p.pos.is_finite() || !p.vel.is_finite() {
                p.pos = old[i];
                p.vel = Vec3::ZERO;
            }
            fastest = fastest.max(p.vel.length());
        }

        if fastest < tu.ragdoll_sleep_speed {
            self.still_time += dt;
            if self.still_time >= tu.ragdoll_sleep_time {
                self.asleep = true;
                for p in &mut self.particles {
                    p.vel = Vec3::ZERO;
                }
            }
        } else {
            self.still_time = 0.0;
        }
    }
}

fn solve_distance(ps: &mut [Particle], a: usize, b: usize, rest: f32) {
    let (wa, wb) = (ps[a].inv_mass, ps[b].inv_mass);
    let w = wa + wb;
    if w <= 0.0 {
        return;
    }
    let d = ps[b].pos - ps[a].pos;
    let len = d.length();
    if len < 1e-9 {
        return;
    }
    let corr = d * ((len - rest) / (len * w));
    ps[a].pos += corr * wa;
    ps[b].pos -= corr * wb;
}

/// Rotate `v` (unit) towards `u` (unit) so the angle between them is `max`.
fn clamp_dir(u: Vec3, v: Vec3, max: f32) -> Option<Vec3> {
    let cos = u.dot(v).clamp(-1.0, 1.0);
    if cos.acos() <= max {
        return None;
    }
    let mut w = (v - u * cos).normalize_or_zero();
    if w.length_sq() < 0.5 {
        w = u
            .cross(if u.y.abs() < 0.9 { Vec3::Y } else { Vec3::X })
            .normalize_or_zero();
    }
    Some(u * max.cos() + w * max.sin())
}

/// Fraction of an angle-limit violation corrected per iteration.
const LIMIT_STIFFNESS: f32 = 0.4;

/// Move `end` by `delta`, split by inverse mass with an equal and opposite move of `pivot`, so
/// limits rotate limbs without pushing the whole body (which would add energy).
fn shift_pair(ps: &mut [Particle], end: usize, pivot: usize, delta: Vec3) {
    let (we, wp) = (ps[end].inv_mass, ps[pivot].inv_mass);
    let w = we + wp;
    if w <= 0.0 {
        return;
    }
    ps[end].pos += delta * (we / w);
    ps[pivot].pos -= delta * (wp / w);
}

fn solve_limit(ps: &mut [Particle], lim: &AngleLimit) {
    match *lim {
        AngleLimit::Cone { a, b, c, d, max } => {
            let u = (ps[b].pos - ps[a].pos).normalize_or_zero();
            let seg = ps[d].pos - ps[c].pos;
            let len = seg.length();
            if len < 1e-6 || u.length_sq() < 0.5 {
                return;
            }
            if let Some(v) = clamp_dir(u, seg / len, max) {
                let target = ps[c].pos + v * len;
                shift_pair(ps, d, c, (target - ps[d].pos) * LIMIT_STIFFNESS);
            }
        }
        AngleLimit::Hinge {
            a,
            b,
            c,
            max,
            frame,
            sign,
        } => {
            let u = (ps[b].pos - ps[a].pos).normalize_or_zero();
            let right = ps[frame[1]].pos - ps[frame[0]].pos;
            let up = ps[frame[3]].pos - ps[frame[2]].pos;
            let fwd = up.cross(right);
            let f = (fwd - u * fwd.dot(u)).normalize_or_zero();
            if u.length_sq() < 0.5 || f.length_sq() < 0.5 {
                return;
            }
            let side = u.cross(f);
            let mut v = ps[c].pos - ps[b].pos;
            let len = v.length();
            if len < 1e-6 {
                return;
            }
            // Keep the bend in the hinge plane and on the allowed side.
            v -= side * v.dot(side);
            let bend = v.dot(f);
            if bend * sign < 0.0 {
                v -= f * bend;
            }
            let mut dir = v.normalize_or_zero();
            if dir.length_sq() < 0.5 {
                dir = u;
            }
            if let Some(cl) = clamp_dir(u, dir, max) {
                dir = cl;
            }
            let target = ps[b].pos + dir * len;
            shift_pair(ps, c, b, (target - ps[c].pos) * LIMIT_STIFFNESS);
        }
    }
}
