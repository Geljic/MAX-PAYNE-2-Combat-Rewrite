//! Bullets are real projectiles with travel time, as in MP2 (visible in bullet time), not
//! hitscan. Each step a projectile sweeps the segment it covers (speed × world dt) and
//! [`crate::Sim`] tests that segment against the world, targets and bodies, and (enemy bullets
//! only) the player. Enemy bullets fly at MP2's `BULLETVELOCITY_ENEMY` (50 m/s), the player's at
//! 150 m/s.

use crate::math::Vec3;
use crate::weapon::Shot;
use crate::world::TargetId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projectile {
    pub pos: Vec3,
    /// Where it was a step ago (for drawing a streak).
    pub prev: Vec3,
    /// Unit flight direction.
    pub dir: Vec3,
    pub speed: f32,
    pub damage: f32,
    pub impulse: f32,
    /// Roster index of the weapon that fired it.
    pub weapon: usize,
    /// Distance flown so far and the most it may fly (m).
    pub travelled: f32,
    pub range: f32,
    /// World seconds alive.
    pub age: f32,
    /// The enemy that fired it; None when the player did. Never hits its own shooter; only enemy
    /// bullets hit the player.
    pub owner: Option<TargetId>,
}

impl Projectile {
    pub fn from_shot(s: &Shot) -> Projectile {
        Projectile {
            pos: s.origin,
            prev: s.origin,
            dir: s.dir,
            speed: s.speed,
            damage: s.damage,
            impulse: s.impulse,
            weapon: s.weapon,
            travelled: 0.0,
            range: s.range,
            age: 0.0,
            owner: None,
        }
    }

    /// Length of this step's sweep for `dt` world seconds, limited by the remaining range.
    pub fn step_length(&self, dt: f32) -> f32 {
        (self.speed * dt).min((self.range - self.travelled).max(0.0))
    }

    /// Move along the sweep by `len`.
    pub fn advance(&mut self, len: f32, dt: f32) {
        self.prev = self.pos;
        self.pos += self.dir * len;
        self.travelled += len;
        self.age += dt;
    }

    pub fn spent(&self, lifetime: f32) -> bool {
        self.travelled >= self.range - 1e-4 || self.age >= lifetime
    }
}
