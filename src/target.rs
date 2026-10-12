//! Shootable characters for testing hits: capsule hit zones, health, zone damage multipliers,
//! flinch / stagger reactions and death (which [`crate::Sim`] hands to a ragdoll).
//!
//! Zone multipliers and health come from MP2's skins (`targetid.h`: head 3, torso 1, arms and legs
//! 0.5; `C01_MAXIMUMHEALTH` 50; `ENEMY_SLOWMOTIONBONUS` 0.12). Capsule sizes and reaction timings
//! are ESTIMATEs (real ones: the skin's target bones and its damage animations).

use crate::anim::{Clip, HitSide};
use crate::math::{Vec3, ray_capsule, yaw_dir, yaw_right};
use crate::world::{HitZone, TargetId};

#[derive(Clone, Debug, PartialEq)]
pub struct TargetDef {
    pub health: f32,
    /// Damage multipliers [head, torso, arm, leg]. targetid.h MULTIPLIER_*.
    pub zone_multiplier: [f32; 4],
    /// Meter given to the killer. Skin SlowMotionBonusForKiller (ENEMY_SLOWMOTIONBONUS).
    pub slowmo_bonus: f32,
    /// Flinch length (s). ESTIMATE: CHARANIM_SMALLDAMAGE.
    pub flinch_time: f32,
    /// Stagger length (s). ESTIMATE: CHARANIM_LARGEDAMAGE.
    pub stagger_time: f32,
    /// Damage within `stagger_window` seconds that causes a stagger. ESTIMATE.
    pub stagger_damage: f32,
    pub stagger_window: f32,
    /// Speed the stagger shoves the body back (m/s). ESTIMATE.
    pub stagger_speed: f32,
    /// Hit capsules in the body frame (x right, y up, z back), radius. ESTIMATE: MP2 tests the
    /// skin's target bones.
    pub zones: Vec<ZoneShape>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneShape {
    pub zone: HitZone,
    pub a: Vec3,
    pub b: Vec3,
    pub radius: f32,
}

impl Default for TargetDef {
    fn default() -> Self {
        let z = |zone, a: [f32; 3], b: [f32; 3], radius| ZoneShape {
            zone,
            a: a.into(),
            b: b.into(),
            radius,
        };
        TargetDef {
            health: 50.0,
            zone_multiplier: [3.0, 1.0, 0.5, 0.5],
            slowmo_bonus: 0.12,
            flinch_time: 0.35,
            stagger_time: 0.8,
            stagger_damage: 40.0,
            stagger_window: 0.1,
            stagger_speed: 1.5,
            zones: vec![
                z(HitZone::Head, [0.0, 1.66, 0.0], [0.0, 1.66, 0.0], 0.12),
                z(HitZone::Torso, [0.0, 1.0, 0.0], [0.0, 1.33, 0.0], 0.2),
                z(HitZone::Arm, [-0.3, 1.4, 0.0], [-0.33, 0.85, 0.0], 0.07),
                z(HitZone::Arm, [0.3, 1.4, 0.0], [0.33, 0.85, 0.0], 0.07),
                z(HitZone::Leg, [-0.11, 0.85, 0.0], [-0.11, 0.08, 0.0], 0.1),
                z(HitZone::Leg, [0.11, 0.85, 0.0], [0.11, 0.08, 0.0], 0.1),
            ],
        }
    }
}

impl TargetDef {
    pub fn multiplier(&self, zone: HitZone) -> f32 {
        self.zone_multiplier[match zone {
            HitZone::Head => 0,
            HitZone::Torso => 1,
            HitZone::Arm => 2,
            HitZone::Leg => 3,
        }]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionKind {
    Flinch,
    Stagger,
    Death,
}

/// How a target reacted to a hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reaction {
    pub kind: ReactionKind,
    pub zone: HitZone,
    /// Side the bullet came from.
    pub from: HitSide,
    /// Horizontal unit direction the body is shoved (along the bullet).
    pub push: Vec3,
    /// Where across the body the bullet struck, -1 (left edge) .. 1 (right edge): which way the
    /// torso twists.
    pub twist: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TargetState {
    Idle,
    Flinch { t: f32, reaction: Reaction },
    Stagger { t: f32, reaction: Reaction },
    Dead,
}

/// Result of one bullet hitting a target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitOutcome {
    pub damage: f32,
    pub reaction: Reaction,
    pub killed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub id: TargetId,
    pub def: TargetDef,
    /// Feet position.
    pub pos: Vec3,
    /// Facing (our yaw convention: 0 faces -Z).
    pub yaw: f32,
    pub health: f32,
    pub state: TargetState,
    /// Horizontal shove velocity from a stagger (m/s).
    pub vel: Vec3,
    /// The enemy brain; None for a plain gallery target.
    pub ai: Option<crate::enemy::Ai>,
    /// Body height scale: 1 standing, less while an enemy crouches (scales hit zones and
    /// [`Target::to_world`] heights).
    pub stance: f32,
    recent_damage: f32,
    since_hit: f32,
}

impl Target {
    pub fn new(id: TargetId, pos: Vec3, yaw: f32, def: TargetDef) -> Target {
        Target {
            id,
            health: def.health,
            def,
            pos,
            yaw,
            state: TargetState::Idle,
            vel: Vec3::ZERO,
            ai: None,
            stance: 1.0,
            recent_damage: 0.0,
            since_hit: 1e6,
        }
    }

    pub fn alive(&self) -> bool {
        !matches!(self.state, TargetState::Dead)
    }

    /// Body-frame point (x right, y up, z back) to world.
    pub fn to_world(&self, p: Vec3) -> Vec3 {
        self.pos + yaw_right(self.yaw) * p.x + Vec3::Y * (p.y * self.stance)
            - yaw_dir(self.yaw) * p.z
    }

    /// World direction to body frame (x right, y up, z back).
    pub fn dir_to_local(&self, d: Vec3) -> Vec3 {
        Vec3::new(d.dot(yaw_right(self.yaw)), d.y, -d.dot(yaw_dir(self.yaw)))
    }

    /// Closest hit zone along a ray: (distance, zone, point, normal).
    pub fn raycast(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_dist: f32,
    ) -> Option<(f32, HitZone, Vec3, Vec3)> {
        if !self.alive() {
            return None;
        }
        let mut best: Option<(f32, HitZone, Vec3, Vec3)> = None;
        for z in &self.def.zones {
            let (a, b) = (self.to_world(z.a), self.to_world(z.b));
            if let Some(t) = ray_capsule(origin, dir, a, b, z.radius)
                && t <= max_dist
                && best.is_none_or(|bb| t < bb.0)
            {
                let p = origin + dir * t;
                let c = crate::world::closest_on_segment(a, b, p);
                best = Some((t, z.zone, p, (p - c).normalize_or_zero()));
            }
        }
        best
    }

    /// Apply one bullet: `base_damage` before the zone multiplier, travelling along `dir`.
    pub fn hit(&mut self, base_damage: f32, zone: HitZone, point: Vec3, dir: Vec3) -> HitOutcome {
        let damage = base_damage * self.def.multiplier(zone);
        let local = self.dir_to_local(-dir); // where the bullet came from
        let from = if local.z.abs() >= local.x.abs() {
            // local z is backwards: a bullet from in front has a negative z source direction.
            if local.z < 0.0 {
                HitSide::Front
            } else {
                HitSide::Back
            }
        } else if local.x > 0.0 {
            HitSide::Right
        } else {
            HitSide::Left
        };
        let lateral = (point - self.pos).dot(yaw_right(self.yaw));
        let mut reaction = Reaction {
            kind: ReactionKind::Flinch,
            zone,
            from,
            push: dir.horizontal().normalize_or_zero(),
            twist: (lateral / 0.35).clamp(-1.0, 1.0),
        };
        if !self.alive() {
            reaction.kind = ReactionKind::Death;
            return HitOutcome {
                damage: 0.0,
                reaction,
                killed: false,
            };
        }
        self.health -= damage;
        if self.since_hit > self.def.stagger_window {
            self.recent_damage = 0.0;
        }
        self.recent_damage += damage;
        self.since_hit = 0.0;
        if self.health <= 0.0 {
            self.health = 0.0;
            reaction.kind = ReactionKind::Death;
            self.state = TargetState::Dead;
            return HitOutcome {
                damage,
                reaction,
                killed: true,
            };
        }
        if self.recent_damage >= self.def.stagger_damage {
            reaction.kind = ReactionKind::Stagger;
            self.state = TargetState::Stagger { t: 0.0, reaction };
            self.vel = reaction.push * self.def.stagger_speed;
        } else if !matches!(self.state, TargetState::Stagger { .. }) {
            self.state = TargetState::Flinch { t: 0.0, reaction };
        }
        HitOutcome {
            damage,
            reaction,
            killed: false,
        }
    }

    /// Advance reactions by `dt` world seconds.
    pub fn step(&mut self, dt: f32) {
        self.since_hit = (self.since_hit + dt).min(1e6);
        match &mut self.state {
            TargetState::Flinch { t, .. } => {
                *t += dt;
                if *t >= self.def.flinch_time {
                    self.state = TargetState::Idle;
                }
            }
            TargetState::Stagger { t, reaction } => {
                *t += dt;
                // Shoved along the bullet, slowing linearly to a stop.
                let u = (*t / self.def.stagger_time).min(1.0);
                self.vel = reaction.push * (self.def.stagger_speed * (1.0 - u));
                self.pos += self.vel * dt;
                if *t >= self.def.stagger_time {
                    self.state = TargetState::Idle;
                    self.vel = Vec3::ZERO;
                }
            }
            _ => {}
        }
    }

    pub fn anim(&self) -> (Clip, f32) {
        match self.state {
            TargetState::Idle => (Clip::TargetIdle, 0.0),
            TargetState::Flinch { t, reaction } => {
                (Clip::TargetFlinch(reaction.zone, reaction.from), t)
            }
            TargetState::Stagger { t, reaction } => (Clip::TargetStagger(reaction.from), t),
            TargetState::Dead => (Clip::Death, 0.0),
        }
    }
}
