//! Animation requests: what the simulation wants a skeleton to play, never how to sample it.
//!
//! Each tick the player reports an [`AnimRequest`]. [`Clip`] names map to MP2's `CHARANIM_*`
//! animations through [`Clip::mp2_name`], so a host with the real clips can look them up; a host
//! with its own rig can match on the enum instead.

use crate::weapon::WeaponClass;
use crate::world::HitZone;

/// Direction relative to the aim (Max always faces the aim in MP2, so moves are directional).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoveDir {
    Forward,
    Back,
    Left,
    Right,
}

impl MoveDir {
    /// Index into `[forward, back, left, right]` move tables.
    pub fn index(self) -> usize {
        self as usize
    }
    /// Classify a local (right, forward) direction.
    pub fn from_local(x: f32, y: f32) -> MoveDir {
        if y.abs() >= x.abs() {
            if y >= 0.0 {
                MoveDir::Forward
            } else {
                MoveDir::Back
            }
        } else if x > 0.0 {
            MoveDir::Right
        } else {
            MoveDir::Left
        }
    }
    fn word(self) -> &'static str {
        match self {
            MoveDir::Forward => "FORWARD",
            MoveDir::Back => "BACKWARD",
            MoveDir::Left => "LEFT",
            MoveDir::Right => "RIGHT",
        }
    }
}

/// Which side a hit came from, relative to the body's facing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitSide {
    Front,
    Back,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Clip {
    // lower body / full body
    Idle,
    Run(MoveDir),
    Crouch,
    CrouchWalk(MoveDir),
    Jump,
    JumpForward,
    Fall,
    Land,
    HardLand,
    Dodge(MoveDir),
    Shootdodge(MoveDir),
    /// Lying after a dive in this direction.
    Prone(MoveDir),
    GetUp(MoveDir),
    Spin,
    Flinch(HitSide),
    Death,
    // upper body
    Aim(WeaponClass),
    Fire(WeaponClass),
    Reload(WeaponClass),
    Switch(WeaponClass),
    // targets
    TargetIdle,
    TargetFlinch(HitZone, HitSide),
    TargetStagger(HitSide),
}

impl Clip {
    /// Best-guess MP2 animation name (`CHARANIM_` stripped). Upper-body and target clips have no
    /// single MP2 name and return a descriptive one.
    pub fn mp2_name(&self) -> String {
        match self {
            Clip::Idle => "STAND".into(),
            Clip::Run(d) => format!("RUN{}", d.word()),
            Clip::Crouch | Clip::CrouchWalk(_) => "CROUCH".into(),
            Clip::Jump => "JUMP".into(),
            Clip::JumpForward => "JUMPFORWARD".into(),
            Clip::Fall => "FALL".into(),
            Clip::Land | Clip::HardLand => "LAND".into(),
            Clip::Dodge(d) => format!("DODGE{}", d.word()),
            Clip::Shootdodge(d) => format!("SHOOTDODGE{}", d.word()),
            Clip::Prone(d) => format!("SHOOTDODGE{}_PRONE", d.word()),
            Clip::GetUp(d) => format!("SHOOTDODGE{}_GETUP", d.word()),
            Clip::Spin => "WHIRLLEFT".into(),
            Clip::Flinch(_) => "SMALLDAMAGE".into(),
            Clip::Death => "RAGDOLLDEATH".into(),
            Clip::Aim(c) => format!("aim_{}", c.name()),
            Clip::Fire(c) => format!("fire_{}", c.name()),
            Clip::Reload(c) => format!("reload_{}", c.name()),
            Clip::Switch(c) => format!("switch_{}", c.name()),
            Clip::TargetIdle => "STAND".into(),
            Clip::TargetFlinch(z, s) => format!("flinch_{z:?}_{s:?}").to_lowercase(),
            Clip::TargetStagger(s) => format!("stagger_{s:?}").to_lowercase(),
        }
    }
}

/// What the player's skeleton should be doing this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimRequest {
    /// Lower / full body clip and seconds into it (player time; restarts when the clip changes).
    pub lower: Clip,
    pub lower_time: f32,
    /// Upper-body layer (aim, fire, reload) and seconds into it, when the state allows one.
    pub upper: Option<(Clip, f32)>,
    /// Body facing (rad, our yaw convention).
    pub body_yaw: f32,
    /// Aim relative to the body: twist the spine / arms this far (rad).
    pub aim_yaw: f32,
    pub aim_pitch: f32,
    /// Ground speed (m/s), to scale locomotion playback.
    pub speed: f32,
    /// Suggested crossfade into a new lower clip (s).
    pub blend: f32,
    /// Seconds per second the player's clock runs (play clips at this rate).
    pub time_scale: f32,
}
