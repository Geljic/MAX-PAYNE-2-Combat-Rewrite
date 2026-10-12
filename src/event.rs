//! One-shot things that happened during a step, for audio, effects and HUD. [`crate::Sim`]
//! collects them; the host drains them with `Sim::drain_events`.

use crate::math::Vec3;
use crate::target::Reaction;
use crate::weapon::Hand;
use crate::world::{HitZone, TargetId};

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A projectile left the muzzle. `weapon` indexes the roster.
    Shot {
        weapon: usize,
        origin: Vec3,
        dir: Vec3,
        hand: Hand,
    },
    /// Trigger pulled on an empty weapon with nothing left to reload.
    DryFire {
        weapon: usize,
    },
    ReloadStarted {
        weapon: usize,
    },
    ReloadDone {
        weapon: usize,
    },
    ReloadCancelled {
        weapon: usize,
    },
    WeaponSwitched {
        from: usize,
        to: usize,
    },
    /// A projectile hit something. `target` is None for level geometry.
    Hit {
        target: Option<TargetId>,
        zone: Option<HitZone>,
        point: Vec3,
        normal: Vec3,
        dir: Vec3,
        damage: f32,
    },
    /// A projectile hit a dead body.
    BodyHit {
        target: Option<TargetId>,
        point: Vec3,
    },
    /// A target reacted to damage (flinch / stagger / death).
    Reaction {
        target: TargetId,
        reaction: Reaction,
    },
    Kill {
        target: TargetId,
        zone: Option<HitZone>,
    },
    Jumped,
    Landed {
        drop: f32,
        hard: bool,
    },
    Dodged,
    ShootdodgeStarted {
        slowmo: bool,
    },
    ShootdodgeLanded,
    GettingUp,
    GotUp,
    Spin,
    BulletTimeStarted,
    BulletTimeEnded {
        ran_out: bool,
    },
    /// Tried to start bullet time with too little meter.
    BulletTimeRefused,
    /// The player took damage. `dir` is the way the hit travelled.
    PlayerHit {
        damage: f32,
        dir: Vec3,
    },
    /// An enemy fired (one event per trigger pull; pellets share it). `dir` is the aim after its
    /// inaccuracy, before the weapon's own scatter.
    EnemyShot {
        enemy: TargetId,
        origin: Vec3,
        dir: Vec3,
    },
    /// An enemy became aware of the player (saw, heard or was told), once per alert.
    EnemyAlerted {
        enemy: TargetId,
    },
    /// The player took a painkiller; `health` is his health right after the immediate part.
    Painkiller {
        health: f32,
    },
    PlayerDied,
    /// An animation message the sim does not act on (sounds, effects), for the host.
    AnimMessage(String),
}
