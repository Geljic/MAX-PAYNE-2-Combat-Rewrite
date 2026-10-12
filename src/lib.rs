//! Max Payne 2 style third-person gunplay, independent of any engine.
//!
//! The core is a fixed-tick state machine. Timings, speeds and weapon stats
//! come from data generated from the player's own copy of Max Payne 2 (see
//! `tools/`), loaded at runtime; anything not yet read from the game is
//! marked ESTIMATE. Bevy integration lives behind `feature = "bevy"`.
//!
//! - [`Sim`]: owns everything; call [`Sim::step`] at 60 Hz with a [`World`] and an [`Input`],
//!   then [`Sim::drain_events`] and [`Sim::anim`].
//! - [`World`]: implement it over your collision, or use [`TestWorld`].
//! - [`Tuning`], [`WeaponDef`], [`MoveSet`]: the numbers; [`GameData::load`] overrides them from
//!   `payne.data`.
//! - [`BulletTime`]: MP2's slow-motion model; [`Ragdoll`]: engine-free bodies.
//! - [`enemy`]: enemy AI ([`Sim::spawn_enemy`]): perception, MP2-style aiming, cover, search.
//!
//! Conventions: Y up, metres, seconds. Yaw 0 faces -Z, positive yaw turns left. (MP2's own
//! data is +Z forward; hosts mapping real clips rotate by 180°.)

pub mod anim;
pub mod bullet_time;
pub mod clips;
pub mod data;
pub mod enemy;
pub mod event;
pub mod math;
pub mod moves;
pub mod player;
pub mod projectile;
pub mod ragdoll;
pub mod sim;
pub mod target;
pub mod tuning;
pub mod weapon;
pub mod world;

#[cfg(feature = "bevy")]
pub mod plugin;

pub use anim::{AnimRequest, Clip, HitSide, MoveDir};
pub use bullet_time::BulletTime;
pub use data::{DataFile, GameData};
pub use enemy::{
    Ai, AiTuning, Difficulty, EnemyAnim, EnemyClass, EnemyData, EnemySkin, Mode, Noise, Style,
    clip_candidates,
};
pub use event::Event;
pub use math::{Quat, Vec3};
pub use moves::{MoveDef, MoveSet, Msg};
pub use player::{Input, Player, State};
pub use projectile::Projectile;
pub use ragdoll::{Ragdoll, SkeletonDef};
pub use sim::{Corpse, Sim};
pub use target::{Reaction, ReactionKind, Target, TargetDef, TargetState};
pub use tuning::Tuning;
pub use weapon::{FireMode, Hand, WeaponClass, WeaponDef, WeaponState};
pub use world::{Aabb, HitZone, RayHit, TargetId, TestWorld, World};

#[cfg(feature = "bevy")]
pub use plugin::PaynePlugin;
