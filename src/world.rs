//! What the simulation needs to know about a level. A host implements [`World`] over its own
//! collision; [`TestWorld`] (a floor plus boxes) serves the tests and the sandbox.
//!
//! The player and projectiles are not part of the world: a host's `raycast` must ignore the
//! player's own collider. Targets owned by [`crate::sim::Sim`] are tested by the sim itself, so a
//! host only reports `target`/`zone` for enemies it simulates on its own.

use crate::math::{Vec3, closest_t};

/// Identifies something that can be shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetId(pub u32);

/// Body region hit by a bullet. MP2 skins define Head, Torso and ArmsAndLegs targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitZone {
    Head,
    Torso,
    Arm,
    Leg,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub point: Vec3,
    pub normal: Vec3,
    /// Distance along the ray (m).
    pub dist: f32,
    pub target: Option<TargetId>,
    pub zone: Option<HitZone>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    pub height: f32,
    pub normal: Vec3,
}

pub trait World {
    /// First hit along a ray (unit `dir`) within `max_dist`.
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit>;

    /// Highest walkable surface at (x, z) whose height is in [y_lo, y_hi]. A height window rather
    /// than a single point so the caller decides how far to step up or snap down.
    fn ground(&self, x: f32, z: f32, y_lo: f32, y_hi: f32) -> Option<Ground>;

    /// Smallest translation that moves a capsule (segment a-b, radius r) out of solid geometry;
    /// zero when it touches nothing. A sphere is a capsule with a == b. Movement steps in short
    /// sub-steps and resolves with this, so no swept test is needed at these speeds.
    fn push_out(&self, a: Vec3, b: Vec3, radius: f32) -> Vec3;

    /// Places an enemy may take cover (feet positions next to low walls, crates and the like). The
    /// AI keeps those hidden from the player; the default has none, so enemies never take cover.
    fn cover_points(&self) -> Vec<Vec3> {
        Vec::new()
    }
}

/// Axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Aabb {
        Aabb { min, max }
    }
    /// Box standing on y = 0 with its footprint centred at (x, z).
    pub fn block(x: f32, z: f32, sx: f32, height: f32, sz: f32) -> Aabb {
        Aabb::new(
            Vec3::new(x - sx * 0.5, 0.0, z - sz * 0.5),
            Vec3::new(x + sx * 0.5, height, z + sz * 0.5),
        )
    }
    pub fn clamp(&self, p: Vec3) -> Vec3 {
        Vec3::new(
            p.x.clamp(self.min.x, self.max.x),
            p.y.clamp(self.min.y, self.max.y),
            p.z.clamp(self.min.z, self.max.z),
        )
    }
    fn dist_sq(&self, p: Vec3) -> f32 {
        (p - self.clamp(p)).length_sq()
    }
}

/// A flat floor (optional) plus boxes.
#[derive(Clone, Debug, Default)]
pub struct TestWorld {
    pub floor: Option<f32>,
    pub boxes: Vec<Aabb>,
}

impl TestWorld {
    /// Endless floor at y = 0.
    pub fn flat() -> TestWorld {
        TestWorld {
            floor: Some(0.0),
            boxes: Vec::new(),
        }
    }

    /// A small shooting range: floor, a low wall to shoot over, a crate and a 1.2 m platform.
    pub fn range() -> TestWorld {
        TestWorld {
            floor: Some(0.0),
            boxes: vec![
                Aabb::block(0.0, -12.0, 6.0, 1.0, 0.5),
                Aabb::block(5.0, -6.0, 1.2, 1.2, 1.2),
                Aabb::block(-7.0, -8.0, 4.0, 1.2, 4.0),
                Aabb::block(0.0, -30.0, 30.0, 4.0, 1.0),
            ],
        }
        .with_gallery()
    }

    /// Add the paper-target lane ([`gallery`]): boards at 5, 10, 15, 25 and 50 m from the firing line.
    pub fn with_gallery(mut self) -> TestWorld {
        self.boxes.extend(gallery::boxes());
        self
    }

    pub fn with_box(mut self, b: Aabb) -> TestWorld {
        self.boxes.push(b);
        self
    }
}

impl TestWorld {
    /// Lowest box height that counts as cover (m). ESTIMATE: hides a crouched enemy's chest.
    pub const COVER_MIN_HEIGHT: f32 = 0.9;
    /// Cover points stand this far out from a box face (m). ESTIMATE: an enemy capsule (0.31) plus
    /// a little.
    pub const COVER_OFFSET: f32 = 0.45;
}

impl World for TestWorld {
    /// Around every box standing on the floor that is at least [`TestWorld::COVER_MIN_HEIGHT`]
    /// tall: points along each face about every metre, [`TestWorld::COVER_OFFSET`] out, dropping
    /// any inside another box.
    fn cover_points(&self) -> Vec<Vec3> {
        let floor = self.floor.unwrap_or(f32::MIN);
        let mut out = Vec::new();
        for b in &self.boxes {
            if b.max.y - b.min.y < Self::COVER_MIN_HEIGHT || b.min.y > floor.max(0.0) + 0.05 {
                continue;
            }
            let o = Self::COVER_OFFSET;
            let y = b.min.y.max(floor);
            let mut along = |from: Vec3, to: Vec3| {
                let n = ((to - from).length().ceil() as usize).max(1);
                for k in 0..=n {
                    out.push(from.lerp(to, k as f32 / n as f32));
                }
            };
            let (x0, x1, z0, z1) = (b.min.x, b.max.x, b.min.z, b.max.z);
            along(Vec3::new(x0, y, z0 - o), Vec3::new(x1, y, z0 - o));
            along(Vec3::new(x0, y, z1 + o), Vec3::new(x1, y, z1 + o));
            along(Vec3::new(x0 - o, y, z0), Vec3::new(x0 - o, y, z1));
            along(Vec3::new(x1 + o, y, z0), Vec3::new(x1 + o, y, z1));
        }
        out.retain(|&p| {
            let a = p + Vec3::Y * 0.5;
            self.push_out(a, a + Vec3::Y, 0.3).horizontal().length() < 1e-4
        });
        out
    }


    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
        let mut best: Option<(f32, Vec3)> = None;
        let mut consider = |t: f32, n: Vec3| {
            if t >= 0.0 && t <= max_dist && best.is_none_or(|(b, _)| t < b) {
                best = Some((t, n));
            }
        };
        if let Some(f) = self.floor
            && dir.y < -1e-9
            && origin.y >= f
        {
            consider((f - origin.y) / dir.y, Vec3::Y);
        }
        for b in &self.boxes {
            if let Some((t, n)) = ray_aabb(origin, dir, b) {
                consider(t, n);
            }
        }
        best.map(|(t, n)| RayHit {
            point: origin + dir * t,
            normal: n,
            dist: t,
            target: None,
            zone: None,
        })
    }

    fn ground(&self, x: f32, z: f32, y_lo: f32, y_hi: f32) -> Option<Ground> {
        let mut best: Option<f32> = None;
        let mut consider = |h: f32| {
            if h >= y_lo && h <= y_hi && best.is_none_or(|b| h > b) {
                best = Some(h);
            }
        };
        if let Some(f) = self.floor {
            consider(f);
        }
        for b in &self.boxes {
            if x >= b.min.x && x <= b.max.x && z >= b.min.z && z <= b.max.z {
                consider(b.max.y);
            }
        }
        best.map(|height| Ground {
            height,
            normal: Vec3::Y,
        })
    }

    fn push_out(&self, a: Vec3, b: Vec3, r: f32) -> Vec3 {
        let mut push = Vec3::ZERO;
        for bx in &self.boxes {
            // Distance from a segment to a box is convex along the segment: ternary search.
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..24 {
                let m1 = lo + (hi - lo) / 3.0;
                let m2 = hi - (hi - lo) / 3.0;
                if bx.dist_sq(a.lerp(b, m1)) < bx.dist_sq(a.lerp(b, m2)) {
                    hi = m2;
                } else {
                    lo = m1;
                }
            }
            let p = a.lerp(b, (lo + hi) * 0.5);
            let q = bx.clamp(p);
            let d = p - q;
            let dl = d.length();
            if dl > 1e-6 {
                if dl < r {
                    push += d * ((r - dl) / dl);
                }
            } else {
                // Centre line inside the box: leave through the nearest face.
                let faces = [
                    (p.x - bx.min.x, Vec3::new(-1.0, 0.0, 0.0)),
                    (bx.max.x - p.x, Vec3::X),
                    (p.y - bx.min.y, Vec3::new(0.0, -1.0, 0.0)),
                    (bx.max.y - p.y, Vec3::Y),
                    (p.z - bx.min.z, Vec3::new(0.0, 0.0, -1.0)),
                    (bx.max.z - p.z, Vec3::Z),
                ];
                let (depth, n) = faces.iter().copied().fold((f32::MAX, Vec3::Y), |acc, f| {
                    if f.0 < acc.0 { f } else { acc }
                });
                push += n * (depth + r);
            }
        }
        if let Some(f) = self.floor {
            let low = a.y.min(b.y) - r;
            if low < f {
                push.y += f - low;
            }
        }
        push
    }
}

/// Slab test. Returns entry distance and face normal.
fn ray_aabb(o: Vec3, d: Vec3, b: &Aabb) -> Option<(f32, Vec3)> {
    let mut tmin = f32::MIN;
    let mut tmax = f32::MAX;
    let mut n = Vec3::ZERO;
    let axes = [
        (o.x, d.x, b.min.x, b.max.x, Vec3::X),
        (o.y, d.y, b.min.y, b.max.y, Vec3::Y),
        (o.z, d.z, b.min.z, b.max.z, Vec3::Z),
    ];
    for (oo, dd, lo, hi, axis) in axes {
        if dd.abs() < 1e-9 {
            if oo < lo || oo > hi {
                return None;
            }
            continue;
        }
        let (mut t1, mut t2) = ((lo - oo) / dd, (hi - oo) / dd);
        let mut face = -axis;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
            face = axis;
        }
        if t1 > tmin {
            tmin = t1;
            n = face;
        }
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }
    if tmax < 0.0 {
        return None;
    }
    Some((tmin.max(0.0), n))
}

/// The shooting-gallery lane: geometry and the scoring maths, so the sandbox, the HUD and tests
/// share one definition. Boards face +Z (towards the firing line); the bullseye is on the face.
pub mod gallery {
    use super::{Aabb, Vec3};

    /// Firing position (x) and firing line (z). The lane runs along -Z, to the right of spawn; far
    /// enough out (x >= 16) to clear the range's 30 m wall.
    pub const LANE_X: f32 = 20.0;
    pub const FIRING_Z: f32 = 0.0;
    /// Board distances from the firing line (m).
    pub const DISTS: [f32; 5] = [5.0, 10.0, 15.0, 25.0, 50.0];
    /// Sideways offset of each board from `LANE_X`. Zigzagged so no board stands in the line of
    /// fire to a farther one, wherever on the firing line you stand near `LANE_X`.
    pub const OFFSETS: [f32; 5] = [-3.6, 3.6, -1.8, 1.8, 0.0];
    pub const BOARD_HALF_W: f32 = 0.8;
    pub const BOARD_HALF_H: f32 = 0.9;
    /// Height of the bullseye centre.
    pub const CENTRE_Y: f32 = 1.4;
    pub const BOARD_THICK: f32 = 0.05;
    /// Ring width (m): ring 10 is the disc of this radius, ring 1 ends at `RING_W * 10`.
    pub const RING_W: f32 = 0.06;
    pub const RINGS: u32 = 10;
    /// How many recent holes per board feed the group size and mean offset.
    pub const GROUP_N: usize = 10;

    /// Bullseye centre on the board face.
    pub fn centre(i: usize) -> Vec3 {
        Vec3::new(LANE_X + OFFSETS[i], CENTRE_Y, FIRING_Z - DISTS[i])
    }

    /// The boards (thin, so bullets stop on them) and their posts.
    pub fn boxes() -> Vec<Aabb> {
        let mut v = Vec::new();
        for i in 0..DISTS.len() {
            let c = centre(i);
            let z = c.z;
            v.push(Aabb::new(
                Vec3::new(c.x - BOARD_HALF_W, CENTRE_Y - BOARD_HALF_H, z - BOARD_THICK),
                Vec3::new(c.x + BOARD_HALF_W, CENTRE_Y + BOARD_HALF_H, z),
            ));
            v.push(Aabb::new(
                Vec3::new(c.x - 0.05, 0.0, z - BOARD_THICK),
                Vec3::new(c.x + 0.05, CENTRE_Y - BOARD_HALF_H, z),
            ));
        }
        v
    }

    /// Score for a hit `r` metres from the centre: 10 in the middle ring down to 1, 0 on the
    /// paper outside the rings.
    pub fn ring_score(r: f32) -> u32 {
        let k = (r / RING_W).floor().max(0.0) as u32;
        RINGS.saturating_sub(k)
    }

    /// Which board a level hit landed on and where: (index, [right, up] from the bullseye in m).
    pub fn board_hit(point: Vec3, normal: Vec3) -> Option<(usize, [f32; 2])> {
        if normal.z < 0.9 {
            return None;
        }
        let i = DISTS
            .iter()
            .position(|d| (point.z - (FIRING_Z - d)).abs() < 0.02)?;
        let (dx, dy) = (point.x - centre(i).x, point.y - CENTRE_Y);
        (dx.abs() <= BOARD_HALF_W && dy.abs() <= BOARD_HALF_H).then_some((i, [dx, dy]))
    }

    /// Group size: the largest distance between any two holes (m). 0 for fewer than two.
    pub fn group_size(p: &[[f32; 2]]) -> f32 {
        let mut m = 0.0f32;
        for (i, a) in p.iter().enumerate() {
            for b in &p[i + 1..] {
                m = m.max((a[0] - b[0]).hypot(a[1] - b[1]));
            }
        }
        m
    }

    /// Mean [right, up] offset of the holes from the bullseye (m); zero when empty.
    pub fn mean_offset(p: &[[f32; 2]]) -> [f32; 2] {
        if p.is_empty() {
            return [0.0, 0.0];
        }
        let n = p.len() as f32;
        [p.iter().map(|a| a[0]).sum::<f32>() / n, p.iter().map(|a| a[1]).sum::<f32>() / n]
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rings_score_by_distance_from_centre() {
            assert_eq!(ring_score(0.0), 10);
            assert_eq!(ring_score(RING_W * 0.99), 10);
            assert_eq!(ring_score(RING_W * 1.01), 9);
            assert_eq!(ring_score(RING_W * 9.5), 1);
            assert_eq!(ring_score(RING_W * 10.5), 0);
            assert_eq!(ring_score(5.0), 0);
        }

        #[test]
        fn group_and_offset_maths() {
            let p = [[0.0, 0.0], [0.03, 0.04], [0.0, 0.01]];
            assert!((group_size(&p) - 0.05).abs() < 1e-6);
            assert_eq!(group_size(&p[..1]), 0.0);
            let m = mean_offset(&[[0.02, 0.0], [0.04, -0.02]]);
            assert!((m[0] - 0.03).abs() < 1e-6 && (m[1] + 0.01).abs() < 1e-6);
        }

        #[test]
        fn finds_the_board_and_the_offset() {
            let c = centre(1);
            let (i, o) = board_hit(c + Vec3::new(0.1, -0.2, 0.0), Vec3::Z).unwrap();
            assert_eq!(i, 1);
            assert!((o[0] - 0.1).abs() < 1e-5 && (o[1] + 0.2).abs() < 1e-5);
            assert!(board_hit(c + Vec3::new(2.0, 0.0, 0.0), Vec3::Z).is_none());
            assert!(board_hit(c, Vec3::Y).is_none());
        }

        #[test]
        fn no_board_blocks_the_line_to_another() {
            use crate::world::{TestWorld, World};
            let w = TestWorld::range();
            let from = Vec3::new(LANE_X, CENTRE_Y, FIRING_Z);
            for i in 0..DISTS.len() {
                let d = (centre(i) - from).normalize_or_zero();
                let h = w.raycast(from, d, 200.0).unwrap();
                assert_eq!(board_hit(h.point, h.normal).map(|b| b.0), Some(i), "board {i}: {h:?}");
            }
        }

        #[test]
        fn a_bullet_stops_on_the_board_face() {
            use crate::world::{TestWorld, World};
            let w = TestWorld::range();
            let o = Vec3::new(centre(0).x, CENTRE_Y, FIRING_Z + 1.0);
            let h = w.raycast(o, Vec3::new(0.0, 0.0, -1.0), 80.0).unwrap();
            assert!((h.dist - 6.0).abs() < 1e-4, "{h:?}");
            assert_eq!(board_hit(h.point, h.normal).unwrap().0, 0);
        }
    }
}

/// Closest point on segment a-b to p (helper for hosts building their own worlds).
pub fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    a.lerp(b, closest_t(a, b, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_box_queries() {
        let w = TestWorld::flat().with_box(Aabb::block(0.0, -5.0, 2.0, 1.0, 2.0));
        let h = w
            .raycast(Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 0.0, -1.0), 50.0)
            .unwrap();
        assert!((h.dist - 4.0).abs() < 1e-4 && h.normal == Vec3::Z);
        assert_eq!(w.ground(0.0, -5.0, -1.0, 2.0).unwrap().height, 1.0);
        assert_eq!(w.ground(0.0, 0.0, -1.0, 2.0).unwrap().height, 0.0);
        let p = w.push_out(Vec3::new(0.0, 0.5, -3.8), Vec3::new(0.0, 0.5, -3.8), 0.3);
        assert!((p.z - 0.1).abs() < 1e-3, "{p:?}");
        let p = w.push_out(Vec3::new(3.0, 0.1, 0.0), Vec3::new(3.0, 0.1, 0.0), 0.3);
        assert!((p.y - 0.2).abs() < 1e-4);
    }
}
