//! Small vector, quaternion and random helpers so the core needs no dependencies.
//!
//! Conventions (same as `parkour`): Y is up, units are metres and seconds. Yaw 0 faces -Z and a
//! positive yaw turns left (counter-clockwise seen from above). Positive pitch looks up.

use std::f32::consts::{PI, TAU};
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Vec3 { x, y, z }
    }
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn length_sq(self) -> f32 {
        self.dot(self)
    }
    pub fn distance(self, o: Vec3) -> f32 {
        (self - o).length()
    }
    /// Unit vector, or zero for (near) zero input.
    pub fn normalize_or_zero(self) -> Vec3 {
        let l = self.length();
        if l < 1e-9 { Vec3::ZERO } else { self / l }
    }
    /// The XZ part (y = 0).
    pub fn horizontal(self) -> Vec3 {
        Vec3::new(self.x, 0.0, self.z)
    }
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
    /// Shorten to at most `max` length.
    pub fn clamp_length(self, max: f32) -> Vec3 {
        let l = self.length();
        if l > max && l > 0.0 {
            self * (max / l)
        } else {
            self
        }
    }
}

impl From<[f32; 3]> for Vec3 {
    fn from(a: [f32; 3]) -> Self {
        Vec3::new(a[0], a[1], a[2])
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Div<f32> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f32) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl MulAssign<f32> for Vec3 {
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}

/// Unit rotation quaternion (x, y, z vector part, w scalar).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
        let a = axis.normalize_or_zero();
        let (s, c) = (angle * 0.5).sin_cos();
        Quat {
            x: a.x * s,
            y: a.y * s,
            z: a.z * s,
            w: c,
        }
    }
    /// Rotation by `yaw` around Y (our yaw convention).
    pub fn from_yaw(yaw: f32) -> Quat {
        Quat::from_axis_angle(Vec3::Y, yaw)
    }
    /// Shortest rotation taking unit vector `from` onto unit vector `to`.
    pub fn from_rotation_arc(from: Vec3, to: Vec3) -> Quat {
        let d = from.dot(to);
        if d < -0.99999 {
            // Opposite: any perpendicular axis.
            let mut axis = Vec3::X.cross(from);
            if axis.length_sq() < 1e-6 {
                axis = Vec3::Y.cross(from);
            }
            return Quat::from_axis_angle(axis, PI);
        }
        let c = from.cross(to);
        Quat {
            x: c.x,
            y: c.y,
            z: c.z,
            w: 1.0 + d,
        }
        .normalize()
    }
    pub fn normalize(self) -> Quat {
        let l = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if l < 1e-9 {
            Quat::IDENTITY
        } else {
            Quat {
                x: self.x / l,
                y: self.y / l,
                z: self.z / l,
                w: self.w / l,
            }
        }
    }
    pub fn conjugate(self) -> Quat {
        Quat {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = q.cross(v) * 2.0;
        v + t * self.w + q.cross(t)
    }
}

impl Mul for Quat {
    type Output = Quat;
    fn mul(self, o: Quat) -> Quat {
        Quat {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }
}

// --- angles ---------------------------------------------------------------------------------

/// Horizontal unit vector a yaw faces.
pub fn yaw_dir(yaw: f32) -> Vec3 {
    Vec3::new(-yaw.sin(), 0.0, -yaw.cos())
}
/// Horizontal unit vector to the right of a yaw (forward × up).
pub fn yaw_right(yaw: f32) -> Vec3 {
    Vec3::new(yaw.cos(), 0.0, -yaw.sin())
}
/// Yaw that faces horizontal direction `d`.
pub fn dir_yaw(d: Vec3) -> f32 {
    (-d.x).atan2(-d.z)
}
/// Unit aim direction from yaw and pitch.
pub fn aim_dir(yaw: f32, pitch: f32) -> Vec3 {
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(-yaw.sin() * cp, sp, -yaw.cos() * cp)
}
/// Pitch of a direction (radians, + up).
pub fn dir_pitch(d: Vec3) -> f32 {
    d.y.atan2(d.horizontal().length())
}
/// Wrap to (-PI, PI].
pub fn wrap_angle(a: f32) -> f32 {
    let r = (a + PI).rem_euclid(TAU) - PI;
    if r <= -PI { r + TAU } else { r }
}
/// Move `value` towards `goal` by at most `max_step`.
pub fn approach(value: f32, goal: f32, max_step: f32) -> f32 {
    value + (goal - value).clamp(-max_step, max_step)
}
/// Move vector `v` towards `goal` by at most `max_step` length.
pub fn approach_vec(v: Vec3, goal: Vec3, max_step: f32) -> Vec3 {
    let d = goal - v;
    let l = d.length();
    if l <= max_step || l < 1e-9 {
        goal
    } else {
        v + d * (max_step / l)
    }
}
/// Turn angle `a` towards `goal` the short way by at most `max_step`.
pub fn approach_angle(a: f32, goal: f32, max_step: f32) -> f32 {
    wrap_angle(a + wrap_angle(goal - a).clamp(-max_step, max_step))
}

// --- geometry -------------------------------------------------------------------------------

/// Closest point to `p` on segment a-b, as the parameter t in [0, 1].
pub fn closest_t(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    if l2 < 1e-12 {
        0.0
    } else {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    }
}

/// Ray (unit `dir`) against a sphere: distance along the ray to the entry point, if hit ahead.
pub fn ray_sphere(origin: Vec3, dir: Vec3, c: Vec3, r: f32) -> Option<f32> {
    let oc = origin - c;
    let b = oc.dot(dir);
    let cc = oc.length_sq() - r * r;
    let h = b * b - cc;
    if h < 0.0 {
        return None;
    }
    let t = -b - h.sqrt();
    if t >= 0.0 {
        Some(t)
    } else if cc <= 0.0 {
        Some(0.0) // started inside
    } else {
        None
    }
}

/// Ray (unit `dir`) against a capsule a-b of radius r: distance to the entry point, if ahead.
pub fn ray_capsule(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, r: f32) -> Option<f32> {
    let ba = b - a;
    let oa = origin - a;
    let baba = ba.length_sq();
    if baba < 1e-10 {
        return ray_sphere(origin, dir, a, r);
    }
    let bard = ba.dot(dir);
    let baoa = ba.dot(oa);
    let rdoa = dir.dot(oa);
    let oaoa = oa.length_sq();
    let qa = baba - bard * bard;
    let qb = baba * rdoa - baoa * bard;
    let qc = baba * oaoa - baoa * baoa - r * r * baba;
    let mut best: Option<f32> = None;
    if qa > 1e-9 {
        let h = qb * qb - qa * qc;
        if h >= 0.0 {
            let t = (-qb - h.sqrt()) / qa;
            let y = baoa + t * bard;
            if t >= 0.0 && y > 0.0 && y < baba {
                best = Some(t);
            }
        }
    }
    // The end caps (also covers rays parallel to the axis).
    for c in [a, b] {
        if let Some(t) = ray_sphere(origin, dir, c, r) {
            best = Some(best.map_or(t, |b: f32| b.min(t)));
        }
    }
    best
}

// --- random ---------------------------------------------------------------------------------

/// Small deterministic generator (xorshift32): spread must replay identically in tests.
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(seed.max(1))
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
    /// A direction within `angle` radians of unit `dir`, roughly uniform over the cone's disc.
    /// MP2's projectile scatter (X_GameObjectsMFC, projectile launch): add a uniformly random unit
    /// vector scaled by `uniform(0, 1) × max` to the direction and renormalise. `max` is the
    /// projectile's `Accuracy / 2500`. Most shots land well inside the maximum.
    pub fn scatter(&mut self, dir: Vec3, max: f32) -> Vec3 {
        if max <= 0.0 {
            return dir;
        }
        let r = loop {
            let v = Vec3::new(
                self.next_f32() * 2.0 - 1.0,
                self.next_f32() * 2.0 - 1.0,
                self.next_f32() * 2.0 - 1.0,
            );
            let l = v.length_sq();
            if l > 0.0 && l <= 1.0 {
                break v.normalize_or_zero();
            }
        };
        (dir + r * (self.next_f32() * max)).normalize_or_zero()
    }

    pub fn cone(&mut self, dir: Vec3, angle: f32) -> Vec3 {
        if angle <= 0.0 {
            return dir;
        }
        let theta = angle * self.next_f32().sqrt();
        let phi = TAU * self.next_f32();
        let helper = if dir.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
        let a = dir.cross(helper).normalize_or_zero();
        let b = dir.cross(a);
        (dir * theta.cos() + (a * phi.cos() + b * phi.sin()) * theta.sin()).normalize_or_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_conventions() {
        assert!((yaw_dir(0.0) - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6);
        assert!((yaw_right(0.0) - Vec3::X).length() < 1e-6);
        let y = 0.7;
        assert!((yaw_dir(y).cross(Vec3::Y) - yaw_right(y)).length() < 1e-6);
        assert!((dir_yaw(yaw_dir(1.2)) - 1.2).abs() < 1e-5);
    }

    #[test]
    fn quat_rotates() {
        let q = Quat::from_rotation_arc(Vec3::Y, Vec3::X);
        assert!((q.rotate(Vec3::Y) - Vec3::X).length() < 1e-5);
        let q = Quat::from_yaw(0.5);
        assert!((q.rotate(yaw_dir(0.0)) - yaw_dir(0.5)).length() < 1e-5);
    }

    #[test]
    fn capsule_ray() {
        let t = ray_capsule(
            Vec3::new(0.0, 1.0, 5.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.0, 1.5, 0.0),
            0.3,
        );
        assert!((t.unwrap() - 4.7).abs() < 1e-4);
        let top = ray_capsule(
            Vec3::new(0.0, 5.0, 0.0),
            -Vec3::Y,
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.0, 1.5, 0.0),
            0.3,
        );
        assert!((top.unwrap() - 3.2).abs() < 1e-4);
    }
}
