//! Enemy AI: engine-free brains for MP2-style gunmen.
//!
//! What comes from MP2's data (`payne.data`, written by `payne-extract` from the player's install):
//! - [`EnemyClass`]: the per-class AI constants of `database/ai_ranges.h` (`[enemyclass C0x]`):
//!   aiming speed, head turning speed, trigger happiness, initial / stable inaccuracy and the time
//!   to settle between them, field of vision, hearing radii and idle timeout.
//! - [`EnemySkin`]: one enemy skin's `[AI]` / `[Properties]` (`[enemyskin NAME]`): its class,
//!   animation style, capsule, kill bonus and shooting frequency multiplier. Hand-tuned skins carry
//!   their own AI keys, which override the class.
//! - [`Difficulty`]: `difficulty.txt` (`[difficulty normal.default]` etc.): inaccuracy multipliers,
//!   painkiller healing and the kill bonus multiplier.
//! - Run and walk speeds: the root-motion curves of the style's clips, looked up in the order
//!   [`clip_candidates`] gives (`SHARP/RUNFORWARD` → `ENEMY/RUNFORWARD` → `RUNFORWARD`).
//!
//! Everything else here (the state machine, cover, search, strafing, alerting nearby enemies) is
//! payne's own design: MP2's real behaviour lives in per-level FSM scripts and AI code we have not
//! read. The numbers for it are in [`AiTuning`], all ESTIMATEs. `docs/behaviour/enemies.md`
//! ("payne's design") describes the state machine.
//!
//! Enemies are [`Target`]s with `ai: Some(..)`: hit zones, health, flinch / stagger / death and
//! the ragdoll corpse all work as for plain targets. Everything an enemy does runs on the world
//! clock, so bullet time slows enemies and their bullets alike.

use std::f32::consts::PI;

use crate::data::{DataFile, Section, parse_number};
use crate::event::Event;
use crate::math::{
    Quat, Rng, Vec3, approach_angle, approach_vec, dir_yaw, yaw_dir, yaw_right,
};
use crate::projectile::Projectile;
use crate::target::{Target, TargetState};
use crate::tuning::Tuning;
use crate::weapon::{Hand, WeaponDef, WeaponState};
use crate::world::{TargetId, World};

// --- data ---------------------------------------------------------------------------------------

/// Which overlay animation set an enemy skin uses (`skins/Male_<Style>.h`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Style {
    /// No overlay: the plain enemy set.
    #[default]
    Plain,
    Normal,
    Sharp,
}

impl Style {
    /// Prefix of the style's clip names: "" | "NORMAL" | "SHARP".
    pub fn name(self) -> &'static str {
        match self {
            Style::Plain => "",
            Style::Normal => "NORMAL",
            Style::Sharp => "SHARP",
        }
    }

    /// Parse a `style =` value (`SHARP`, `NORMAL`, anything else plain).
    pub fn parse(s: &str) -> Style {
        match s.trim().to_ascii_uppercase().as_str() {
            "SHARP" => Style::Sharp,
            "NORMAL" => Style::Normal,
            _ => Style::Plain,
        }
    }
}

/// Clip names to try, in order, for an animation slot of an enemy of `style`:
/// `S/<SLOT>` → `ENEMY/<SLOT>` → `<SLOT>`. A plain enemy has no style set, so its list is
/// `ENEMY/<SLOT>`, `<SLOT>`, `<SLOT>` (the last entry repeats).
pub fn clip_candidates(style: Style, slot: &str) -> [String; 3] {
    let enemy = format!("ENEMY/{slot}");
    match style {
        Style::Plain => [enemy, slot.to_string(), slot.to_string()],
        s => [format!("{}/{slot}", s.name()), enemy, slot.to_string()],
    }
}

/// Per-class AI constants (`ai_ranges.h`, `[enemyclass C0x]`). Defaults are class C01 (Cleaners,
/// Mafiosos) as read from the retail `ai_ranges.h`, so the AI works without `payne.data`.
#[derive(Clone, Debug, PartialEq)]
pub struct EnemyClass {
    /// `C00` .. `C05`.
    pub name: String,
    /// `C0x_MAXIMUMHEALTH`.
    pub health: f32,
    /// How fast the aim turns (deg/s). AimingSpeed.
    pub aiming_speed: f32,
    /// How fast the head (here: the body) turns towards what it looks at (deg/s). HeadTurningSpeed.
    pub head_turning_speed: f32,
    /// Starts shooting once the aim is within this many degrees of the target. TriggerHappinessAngle.
    pub trigger_happiness_angle: f32,
    /// Metres of error per 25 m when it starts aiming. InitialInaccuracy.
    pub initial_inaccuracy: f32,
    /// Metres of error per 25 m once settled. StableInaccuracy.
    pub stable_inaccuracy: f32,
    /// Seconds to settle from initial to stable. StabilizationTime.
    pub stabilization_time: f32,
    /// Degrees, centred on the facing. FieldOfVision.
    pub field_of_vision: f32,
    /// Hearing radius per loudness group one / two / three (m). HearingGroup*Radius.
    pub hearing_radius: [f32; 3],
    /// Seconds without news of the player before going back to idle. IdleTimeout.
    pub idle_timeout: f32,
}

impl Default for EnemyClass {
    fn default() -> Self {
        EnemyClass {
            name: "C01".into(),
            health: 50.0,
            aiming_speed: 240.0,
            head_turning_speed: 360.0,
            trigger_happiness_angle: 35.0,
            initial_inaccuracy: 4.0,
            stable_inaccuracy: 2.5,
            stabilization_time: 3.0,
            field_of_vision: 150.0,
            hearing_radius: [1000.0, 5.0, 2.0],
            idle_timeout: 10.0,
        }
    }
}

impl EnemyClass {
    /// Override values from a section using the `[enemyclass]` key names.
    pub fn apply(&mut self, s: &Section) {
        let set = |v: &mut f32, k: &str| {
            if let Some(n) = s.num(k) {
                *v = n;
            }
        };
        set(&mut self.health, "health");
        set(&mut self.aiming_speed, "aiming_speed");
        set(&mut self.head_turning_speed, "head_turning_speed");
        set(&mut self.trigger_happiness_angle, "trigger_happiness_angle");
        set(&mut self.initial_inaccuracy, "initial_inaccuracy");
        set(&mut self.stable_inaccuracy, "stable_inaccuracy");
        set(&mut self.stabilization_time, "stabilization_time");
        set(&mut self.field_of_vision, "field_of_vision");
        set(&mut self.idle_timeout, "idle_timeout");
        if let Some(h) = s.get("hearing_radius") {
            for (i, v) in h.split_whitespace().filter_map(parse_number).take(3).enumerate() {
                self.hearing_radius[i] = v;
            }
        }
    }
}

/// One enemy skin (`[enemyskin NAME]`), with its class already resolved. Defaults are
/// `C01_Mafioso_A` from the retail skins (capsule 0.31 / 1.95, `ENEMY_SLOWMOTIONBONUS` 0.12).
#[derive(Clone, Debug, PartialEq)]
pub struct EnemySkin {
    pub name: String,
    /// Class values, with any AI keys of the skin itself applied on top.
    pub class: EnemyClass,
    pub style: Style,
    /// Body capsule radius and top (m). CapsuleRadius / CapsuleTop.
    pub capsule_radius: f32,
    pub capsule_top: f32,
    /// Bullet-time meter the killer gains. SlowMotionBonusForKiller.
    pub slowmo_bonus: f32,
    /// Multiplier on the weapon's fire rate. ShootingFrequencyMultiplier.
    pub shooting_frequency_multiplier: f32,
    /// AirborneSpeed (stored; enemies do not jump yet).
    pub airborne_speed: f32,
    /// Shootdodge frames (stored; enemies do not shootdodge yet).
    pub shootdodge_frames: [f32; 2],
}

impl Default for EnemySkin {
    fn default() -> Self {
        EnemySkin {
            name: "C01_Mafioso_A".into(),
            class: EnemyClass::default(),
            style: Style::Plain,
            capsule_radius: 0.31,
            capsule_top: 1.95,
            slowmo_bonus: 0.12,
            shooting_frequency_multiplier: 1.0,
            airborne_speed: 2.5,
            shootdodge_frames: [10.0, 45.0],
        }
    }
}

/// Difficulty values (`difficulty.txt`). Defaults are `normal.default`, the retail normal level.
/// MP2 slides between a mode's minimum and maximum as the player kills and dies; payne uses one
/// fixed level (pick another from [`crate::GameData::difficulties`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Difficulty {
    /// `<mode>.<level>`, e.g. `normal.default`.
    pub name: String,
    /// Multiplies [`EnemyClass::initial_inaccuracy`]. InitialInaccuracyMultiplier.
    pub initial_inaccuracy_multiplier: f32,
    /// Multiplies [`EnemyClass::stable_inaccuracy`]. StableInaccuracyMultiplier.
    pub stable_inaccuracy_multiplier: f32,
    /// Painkiller regeneration rate, fraction of maximum health per second. HealthRegenerateMultiplier.
    pub health_regenerate_multiplier: f32,
    /// Fraction of maximum health a painkiller gives at once. HealthGainedImmediately.
    pub health_gained_immediately: f32,
    /// Fraction of maximum health a painkiller gives in all. HealthGainedFromPainkiller.
    pub health_gained_from_painkiller: f32,
    /// Multiplies the victim's kill bonus. BulletTimeGainedPerKillMultiplier.
    pub bullet_time_gained_per_kill_multiplier: f32,
    /// Stored, unused: payne has no auto-aim or pickups yet.
    pub auto_aim_capsule_radius_multiplier: f32,
    pub auto_aim_capsule_height_multiplier: f32,
    pub ammo_found_multiplier: f32,
    /// Below this fraction of maximum health the player heals by itself up to it. From the mode's
    /// `.properties` AutoHealLevel (ASSUMPTION about its meaning).
    pub auto_heal_level: f32,
    /// Auto-heal rate, fraction of maximum health per second. AutoHealHealthIncreasePerSecond.
    pub auto_heal_per_second: f32,
}

impl Default for Difficulty {
    fn default() -> Self {
        Difficulty {
            name: "normal.default".into(),
            initial_inaccuracy_multiplier: 1.3,
            stable_inaccuracy_multiplier: 1.3,
            health_regenerate_multiplier: 0.04,
            health_gained_immediately: 0.05,
            health_gained_from_painkiller: 0.2,
            bullet_time_gained_per_kill_multiplier: 1.0,
            auto_aim_capsule_radius_multiplier: 4.0,
            auto_aim_capsule_height_multiplier: 2.0,
            ammo_found_multiplier: 1.0,
            auto_heal_level: 0.15,
            auto_heal_per_second: 0.01,
        }
    }
}

impl Difficulty {
    /// The level `name` (`normal.default`) from a file's `[difficulty]` sections, with its mode's
    /// `.properties`. None if the file has no such section.
    pub fn from_file(file: &DataFile, name: &str) -> Option<Difficulty> {
        let s = file.section("difficulty", name)?;
        let mut d = Difficulty {
            name: s.name.clone(),
            ..Difficulty::default()
        };
        d.apply(s);
        let mode = name.split('.').next().unwrap_or(name);
        if let Some(p) = file.section("difficulty", &format!("{mode}.properties")) {
            d.apply(p);
        }
        Some(d)
    }

    /// Every level in a file (all `[difficulty X.Y]` sections except `.properties`).
    pub fn all(file: &DataFile) -> Vec<Difficulty> {
        file.of_kind("difficulty")
            .filter(|s| !s.name.to_ascii_lowercase().ends_with(".properties"))
            .filter_map(|s| Difficulty::from_file(file, &s.name))
            .collect()
    }

    fn apply(&mut self, s: &Section) {
        let set = |v: &mut f32, k: &str| {
            if let Some(n) = s.num(k) {
                *v = n;
            }
        };
        set(&mut self.initial_inaccuracy_multiplier, "initial_inaccuracy_multiplier");
        set(&mut self.stable_inaccuracy_multiplier, "stable_inaccuracy_multiplier");
        set(&mut self.health_regenerate_multiplier, "health_regenerate_multiplier");
        set(&mut self.health_gained_immediately, "health_gained_immediately");
        set(&mut self.health_gained_from_painkiller, "health_gained_from_painkiller");
        set(
            &mut self.bullet_time_gained_per_kill_multiplier,
            "bullet_time_gained_per_kill_multiplier",
        );
        set(&mut self.auto_aim_capsule_radius_multiplier, "auto_aim_capsule_radius_multiplier");
        set(&mut self.auto_aim_capsule_height_multiplier, "auto_aim_capsule_height_multiplier");
        set(&mut self.ammo_found_multiplier, "ammo_found_multiplier");
        set(&mut self.auto_heal_level, "auto_heal_level");
        set(&mut self.auto_heal_per_second, "auto_heal_health_increase_per_second");
    }
}

/// payne's own AI numbers. All ESTIMATEs: MP2's real behaviour lives in level FSM scripts and the
/// AI code (X_GameObjectsMFC), which we have not read.
#[derive(Clone, Debug, PartialEq)]
pub struct AiTuning {
    /// Eye height above the feet, standing (m). ESTIMATE: about the male skeleton's head bone.
    pub eye_height: f32,
    /// Crouched behind cover, the body (eyes, hit zones, capsule) shrinks to this fraction. ESTIMATE:
    /// the CROUCH pose's head height over the standing one.
    pub crouch_scale: f32,
    /// Farthest the eyes see (m). ESTIMATE.
    pub sight_range: f32,
    /// An enemy that spots the player alerts every enemy within this radius (m). ESTIMATE.
    pub alert_radius: f32,
    /// Seconds after losing sight before going to search the last known position. ESTIMATE.
    pub lose_sight_time: f32,
    /// Out of sight this long, the aim restarts from initial inaccuracy when the player is seen
    /// again (s). ESTIMATE.
    pub reacquire_time: f32,
    /// Alerted by a noise: seconds spent looking towards it before going to search there. ESTIMATE.
    pub alert_look_time: f32,
    /// Seconds between sidesteps in a firefight (random in min..max), and how long one lasts.
    /// ESTIMATE. `strafe_time_max <= 0` disables strafing.
    pub strafe_interval_min: f32,
    pub strafe_interval_max: f32,
    pub strafe_time_min: f32,
    pub strafe_time_max: f32,
    /// Cover farther than this is not considered (m). ESTIMATE.
    pub cover_radius: f32,
    /// Seconds before the enemy may look for cover again. ESTIMATE.
    pub cover_cooldown: f32,
    /// Height of the point that must be hidden from the player for cover to count (m). ESTIMATE.
    pub cover_test_height: f32,
    /// Score penalty (m) for cover the enemy cannot shoot over when standing up. ESTIMATE.
    pub full_cover_penalty: f32,
    /// Crouched behind cover for this long, then stand up and shoot for `peek_time` (s). ESTIMATE.
    pub hide_time: f32,
    pub peek_time: f32,
    /// After this many peeks without seeing the player, go and search for him. ESTIMATE.
    pub max_unseen_peeks: u32,
    /// Hold fire while another enemy's body is this close to the line of fire (m). ESTIMATE.
    pub ally_clearance: f32,
    /// A player bullet passing within this distance of the chest counts as being shot at (m). ESTIMATE.
    pub near_miss_radius: f32,
    /// Ground speed above which the player's footsteps are loud (hearing group two); slower moving is
    /// group three (m/s). ESTIMATE: MP2 makes footstep noise with pressurewave projectiles on the
    /// run animations' footstep frames.
    pub loud_footstep_speed: f32,
    /// Fallback speeds when no clip curve is loaded (m/s). ESTIMATE (Max's measured run is 5.25).
    pub run_speed: f32,
    pub strafe_speed: f32,
    pub backpedal_speed: f32,
    pub walk_speed: f32,
    /// Acceleration towards the wanted speed (m/s²). ESTIMATE.
    pub accel: f32,
}

impl Default for AiTuning {
    fn default() -> Self {
        AiTuning {
            eye_height: 1.6,
            crouch_scale: 0.7,
            sight_range: 100.0,
            alert_radius: 15.0,
            lose_sight_time: 2.0,
            reacquire_time: 1.0,
            alert_look_time: 1.0,
            strafe_interval_min: 2.0,
            strafe_interval_max: 5.0,
            strafe_time_min: 0.5,
            strafe_time_max: 1.0,
            cover_radius: 12.0,
            cover_cooldown: 4.0,
            cover_test_height: 0.9,
            full_cover_penalty: 6.0,
            hide_time: 1.5,
            peek_time: 2.5,
            max_unseen_peeks: 2,
            ally_clearance: 0.5,
            near_miss_radius: 1.5,
            loud_footstep_speed: 3.0,
            run_speed: 5.0,
            strafe_speed: 4.6,
            backpedal_speed: 3.75,
            walk_speed: 1.2,
            accel: 30.0,
        }
    }
}

/// Everything enemies take from game data, plus payne's AI tuning.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnemyData {
    pub classes: Vec<EnemyClass>,
    pub skins: Vec<EnemySkin>,
    pub tuning: AiTuning,
    /// Average root-motion speed (m/s) of every clip with a root curve, by clip name (e.g.
    /// `SHARP/RUNFORWARD`). From `[anim]` root curves, or [`EnemyData::measure_speeds`].
    pub clip_speeds: Vec<(String, f32)>,
}

/// Frame rate assumed for a `root =` curve whose section has no `fps` (the measured block does not
/// write one). ESTIMATE: Max's measured run cycles play at 50 fps, the walk cycles at 30; the
/// exact rate is in anims.bin ([`EnemyData::measure_speeds`]).
fn assumed_root_fps(name: &str) -> f32 {
    if name.contains("WALK") { 30.0 } else { 50.0 }
}

impl EnemyData {
    /// Read `[enemyclass]`, `[enemyskin]` and the root curves of `[anim]` sections.
    pub fn from_file(file: &DataFile) -> EnemyData {
        let mut d = EnemyData::default();
        for s in file.of_kind("enemyclass") {
            let mut c = EnemyClass {
                name: s.name.clone(),
                ..EnemyClass::default()
            };
            c.apply(s);
            d.classes.push(c);
        }
        for s in file.of_kind("enemyskin") {
            let class_name = s.get("class").unwrap_or("C01").trim();
            let mut class = d.class(class_name);
            class.apply(s); // hand-tuned skins carry their own AI keys
            let mut k = EnemySkin {
                name: s.name.clone(),
                class,
                style: s.get("style").map(Style::parse).unwrap_or_default(),
                ..EnemySkin::default()
            };
            let set = |v: &mut f32, key: &str| {
                if let Some(n) = s.num(key) {
                    *v = n;
                }
            };
            set(&mut k.capsule_radius, "capsule_radius");
            set(&mut k.capsule_top, "capsule_top");
            set(&mut k.slowmo_bonus, "slowmo_bonus");
            set(&mut k.shooting_frequency_multiplier, "shooting_frequency_multiplier");
            set(&mut k.airborne_speed, "airborne_speed");
            if let Some(f) = s.get("shootdodge_frames") {
                let v: Vec<f32> = f.split_whitespace().filter_map(parse_number).collect();
                if let [a, b] = v[..] {
                    k.shootdodge_frames = [a, b];
                }
            }
            d.skins.push(k);
        }
        for s in file.of_kind("anim") {
            let Some(r) = s.get("root") else { continue };
            let curve: Vec<f32> = r.split_whitespace().filter_map(parse_number).collect();
            if curve.len() < 2 {
                continue;
            }
            let name = s.name.to_ascii_uppercase();
            let fps = s.num("fps").unwrap_or_else(|| assumed_root_fps(&name));
            let secs = (curve.len() - 1) as f32 / fps.max(1e-6);
            let speed = (curve[curve.len() - 1] - curve[0]) / secs;
            d.clip_speeds.push((name, speed));
        }
        d
    }

    /// Replace clip speeds with the exact ones from baked clips (their root tracks carry the frame
    /// rate, which `payne.data` does not). Call after loading `anims.bin`.
    pub fn measure_speeds(&mut self, set: &crate::clips::AnimSet) {
        for c in &set.clips {
            let Some(r) = &c.root else { continue };
            if r.frames.len() < 2 || r.fps <= 0.0 {
                continue;
            }
            let last = r.frames[r.frames.len() - 1];
            let secs = (r.frames.len() - 1) as f32 / r.fps;
            let speed = last[0].hypot(last[2]) / secs;
            let name = c.name.to_ascii_uppercase();
            match self.clip_speeds.iter_mut().find(|(n, _)| *n == name) {
                Some(e) => e.1 = speed,
                None => self.clip_speeds.push((name, speed)),
            }
        }
    }

    /// A class by name, or the C01 defaults.
    pub fn class(&self, name: &str) -> EnemyClass {
        self.classes
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .cloned()
            .unwrap_or_default()
    }

    /// A skin by name, or the C01_Mafioso_A defaults (with the loaded C01 class, if any) named `name`.
    pub fn skin(&self, name: &str) -> EnemySkin {
        self.skins
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .cloned()
            .unwrap_or_else(|| EnemySkin {
                name: name.to_string(),
                class: self.class("C01"),
                ..EnemySkin::default()
            })
    }

    /// Root-motion speed of a slot for a style, through [`clip_candidates`].
    pub fn clip_speed(&self, style: Style, slot: &str) -> Option<f32> {
        clip_candidates(style, slot).iter().find_map(|n| {
            self.clip_speeds
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(n))
                .map(|(_, v)| *v)
                .filter(|v| *v > 0.05)
        })
    }

    fn speeds(&self, style: Style) -> Speeds {
        let t = &self.tuning;
        Speeds {
            run: self.clip_speed(style, "RUNFORWARD").unwrap_or(t.run_speed),
            strafe: self.clip_speed(style, "RUNLEFT").unwrap_or(t.strafe_speed),
            back: self.clip_speed(style, "RUNBACKWARD").unwrap_or(t.backpedal_speed),
            walk: self.clip_speed(style, "WALKFORWARD").unwrap_or(t.walk_speed),
        }
    }
}

// --- the brain ----------------------------------------------------------------------------------

/// Ground speeds an enemy moves at (m/s), from its style's clips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speeds {
    pub run: f32,
    pub strafe: f32,
    pub back: f32,
    pub walk: f32,
}

/// What the brain is doing. Times are world seconds in the mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// Unaware: standing at its post.
    Idle,
    /// Heard something (or was told): turns to look at `look_at`.
    Alerted { look_at: Vec3, t: f32 },
    /// Sees (or just saw) the player: holds position, aims, shoots, sidesteps now and then.
    Combat,
    /// Running to a cover point.
    TakeCover { to: Vec3, t: f32 },
    /// Behind cover: crouched, then standing up (`peeking`) to shoot.
    InCover { t: f32, peeking: bool, unseen_peeks: u32 },
    /// Going to where the player was last seen or heard, then looking around there.
    Search { to: Vec3, t: f32, arrived: bool },
}

impl Mode {
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Idle => "idle",
            Mode::Alerted { .. } => "alerted",
            Mode::Combat => "combat",
            Mode::TakeCover { .. } => "take cover",
            Mode::InCover { peeking: true, .. } => "peek",
            Mode::InCover { .. } => "in cover",
            Mode::Search { .. } => "search",
        }
    }
}

/// One enemy's brain and gun. Public state is for HUDs, debugging and tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Ai {
    pub skin: EnemySkin,
    pub mode: Mode,
    /// The carried weapon (`def` indexes the sim's roster). Spare rounds never run out.
    pub weapon: WeaponState,
    /// Where the gun points (unit). Turns at the class's aiming speed.
    pub aim_dir: Vec3,
    /// Angle between `aim_dir` and the player while seen (rad).
    pub aim_error: f32,
    /// Seconds spent aiming at the player since acquiring him: drives inaccuracy settling.
    pub aim_time: f32,
    /// Sees the player this tick.
    pub sees: bool,
    pub since_seen: f32,
    /// Where the player (his chest) was last seen, heard or reported.
    pub last_seen: Option<Vec3>,
    /// Seconds since any news of the player (sight, sound, alert, being shot).
    pub since_stimulus: f32,
    /// Has been alerted (EnemyAlerted sent) since it was last idle.
    pub alerted: bool,
    pub crouched: bool,
    /// Horizontal velocity (m/s).
    pub vel: Vec3,
    pub speeds: Speeds,
    /// Seconds until the next shot may fire.
    pub cooldown: f32,
    strafe_dir: f32,
    strafe_left: f32,
    next_strafe: f32,
    cover_cooldown: f32,
    /// Shot at from here this tick (set by the sim).
    threat: Option<Vec3>,
    /// Heard or told about the player at this point this tick.
    news: Option<Vec3>,
    vy: f32,
    airborne: bool,
    /// The right-hand muzzle last tick (for the animation's aim point).
    gun: Vec3,
    anim_slot: &'static str,
    anim_t: f32,
}

impl Ai {
    pub fn new(skin: EnemySkin, data: &EnemyData, weapon: usize, def: &WeaponDef, yaw: f32) -> Ai {
        let speeds = data.speeds(skin.style);
        Ai {
            skin,
            mode: Mode::Idle,
            weapon: WeaponState::new(weapon, def, def.pocket),
            aim_dir: yaw_dir(yaw),
            aim_error: PI,
            aim_time: 0.0,
            sees: false,
            since_seen: 1e6,
            last_seen: None,
            since_stimulus: 1e6,
            alerted: false,
            crouched: false,
            vel: Vec3::ZERO,
            speeds,
            cooldown: 0.0,
            strafe_dir: 0.0,
            strafe_left: 0.0,
            next_strafe: 2.0,
            cover_cooldown: 0.0,
            threat: None,
            news: None,
            vy: 0.0,
            airborne: false,
            gun: Vec3::ZERO,
            anim_slot: "STAND",
            anim_t: 0.0,
        }
    }

    /// Current inaccuracy: metres of error per 25 m, settling linearly from the initial to the
    /// stable value over the class's stabilization time, times the difficulty multipliers.
    pub fn inaccuracy(&self, d: &Difficulty) -> f32 {
        let c = &self.skin.class;
        let k = if c.stabilization_time > 0.0 {
            (self.aim_time / c.stabilization_time).min(1.0)
        } else {
            1.0
        };
        let a = c.initial_inaccuracy * d.initial_inaccuracy_multiplier;
        let b = c.stable_inaccuracy * d.stable_inaccuracy_multiplier;
        a + (b - a) * k
    }

    /// Being shot at (hit or near miss) from `from`.
    pub fn on_shot_at(&mut self, from: Vec3) {
        self.threat = Some(from);
    }

    /// News of the player at `at` (a noise, an ally's alert).
    pub fn on_news(&mut self, at: Vec3) {
        self.news = Some(at);
    }
}

/// A sound enemies may hear. `group` 0..3 is MP2's loudness group one..three.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Noise {
    pub pos: Vec3,
    pub group: usize,
    /// Where a listener should look: the player's chest for his own noises, the shooter's target
    /// for an enemy's gunshot.
    pub interest: Vec3,
    /// The enemy that made it (it does not hear itself).
    pub source: Option<TargetId>,
}

/// What enemies can perceive of the player this tick.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sense {
    pub alive: bool,
    pub chest: Vec3,
    /// Points that count as seeing him: chest and the top of his capsule.
    pub points: [Vec3; 2],
}

pub(crate) struct EnemyCtx<'a> {
    pub tuning: &'a Tuning,
    pub data: &'a EnemyData,
    pub difficulty: &'a Difficulty,
    pub weapons: &'a [WeaponDef],
    pub sense: Sense,
    pub noises: &'a [Noise],
    /// Cover points in the world, fetched lazily.
    pub covers: &'a mut Option<Vec<Vec3>>,
    /// Where the other enemies are and are going (id, point), so two do not take one cover.
    pub claimed: &'a [(TargetId, Vec3)],
    /// Feet of every live enemy (id, position), to hold fire when one stands in the way.
    pub allies: &'a [(TargetId, Vec3)],
    pub rng: &'a mut Rng,
    pub events: &'a mut Vec<Event>,
    pub shots: &'a mut Vec<Projectile>,
    pub noises_out: &'a mut Vec<Noise>,
    /// (alerter, its position, where the player is) for spreading the alarm.
    pub alerts: &'a mut Vec<(TargetId, Vec3, Vec3)>,
    /// World seconds this tick.
    pub dt: f32,
}

fn rand_range(rng: &mut Rng, a: f32, b: f32) -> f32 {
    a + (b - a) * rng.next_f32()
}

/// Turn unit `a` towards unit `b` by at most `max` radians.
pub fn turn_towards(a: Vec3, b: Vec3, max: f32) -> Vec3 {
    let ang = a.dot(b).clamp(-1.0, 1.0).acos();
    if ang <= max || ang < 1e-6 {
        return b;
    }
    let s = ang.sin();
    if s < 1e-4 {
        // Opposite: swing round the vertical.
        return Quat::from_axis_angle(Vec3::Y, max).rotate(a).normalize_or_zero();
    }
    let t = max / ang;
    (a * (((1.0 - t) * ang).sin() / s) + b * ((t * ang).sin() / s)).normalize_or_zero()
}

fn angle_between(a: Vec3, b: Vec3) -> f32 {
    a.normalize_or_zero()
        .dot(b.normalize_or_zero())
        .clamp(-1.0, 1.0)
        .acos()
}

/// Is the straight line a → b blocked by level geometry?
fn blocked<W: World + ?Sized>(world: &W, a: Vec3, b: Vec3) -> bool {
    let d = b - a;
    let l = d.length();
    l > 1e-4 && world.raycast(a, d / l, l).is_some_and(|h| h.dist < l - 0.05)
}

fn stance(ai: &Ai, at: &AiTuning) -> f32 {
    if ai.crouched { at.crouch_scale } else { 1.0 }
}

/// Muzzle of an enemy: Max's measured gun-bone heights (same male skeleton) and side offset.
pub(crate) fn muzzle(t: &Target, ai: &Ai, tu: &Tuning, hand: Hand) -> Vec3 {
    let h = if ai.crouched { tu.muzzle_crouch } else { tu.muzzle_stand };
    let side = if hand == Hand::Right { 1.0 } else { -1.0 };
    t.pos + Vec3::Y * h + yaw_right(t.yaw) * (tu.muzzle_side * side)
}

fn can_see<W: World + ?Sized>(t: &Target, ai: &Ai, world: &W, s: &Sense, at: &AiTuning) -> bool {
    if !s.alive {
        return false;
    }
    let eye = t.pos + Vec3::Y * (at.eye_height * stance(ai, at));
    let to = s.chest - eye;
    if to.length() > at.sight_range {
        return false;
    }
    let h = to.horizontal().normalize_or_zero();
    let half = (ai.skin.class.field_of_vision * 0.5).to_radians();
    if h.length_sq() > 0.0 && yaw_dir(t.yaw).dot(h) < half.cos() {
        return false;
    }
    s.points.iter().any(|&p| !blocked(world, eye, p))
}

/// Best cover point from `threat`: hidden at `cover_test_height`, preferably clear to shoot over
/// when standing, nearest first.
fn find_cover<W: World + ?Sized>(
    t: &Target,
    world: &W,
    threat: Vec3,
    cx: &mut EnemyCtx,
) -> Option<Vec3> {
    let at = &cx.data.tuning;
    let covers = cx.covers.get_or_insert_with(|| world.cover_points());
    let mut best: Option<(f32, Vec3)> = None;
    for &c in covers.iter() {
        let dist = c.horizontal().distance(t.pos.horizontal());
        if dist > at.cover_radius {
            continue;
        }
        if cx
            .claimed
            .iter()
            .any(|(id, p)| *id != t.id && p.horizontal().distance(c.horizontal()) < 0.8)
        {
            continue;
        }
        if !blocked(world, threat, c + Vec3::Y * at.cover_test_height) {
            continue;
        }
        let peek = !blocked(world, threat, c + Vec3::Y * at.eye_height);
        let score = dist + if peek { 0.0 } else { at.full_cover_penalty };
        if best.is_none_or(|b| score < b.0) {
            best = Some((score, c));
        }
    }
    best.map(|b| b.1)
}

fn announce(t: &Target, ai: &mut Ai, cx: &mut EnemyCtx) {
    if !ai.alerted {
        ai.alerted = true;
        cx.events.push(Event::EnemyAlerted { enemy: t.id });
    }
}

/// One enemy's tick on the world clock.
pub(crate) fn step<W: World + ?Sized>(t: &mut Target, world: &W, cx: &mut EnemyCtx) {
    if !t.alive() {
        return;
    }
    let Some(mut ai) = t.ai.take() else { return };
    let dt = cx.dt;
    let at = &cx.data.tuning;
    let tu = cx.tuning;
    let class = ai.skin.class.clone();
    let def = &cx.weapons[ai.weapon.def.min(cx.weapons.len() - 1)];
    if ai.weapon.tick(def, dt) {
        ai.weapon.reserve = def.pocket; // endless spare rounds
    }
    ai.cooldown = (ai.cooldown - dt).max(-1.0);
    ai.cover_cooldown -= dt;
    ai.since_stimulus = (ai.since_stimulus + dt).min(1e6);
    let reacting = matches!(t.state, TargetState::Flinch { .. } | TargetState::Stagger { .. });

    // --- perception ---
    let was_seeing = ai.sees;
    let unseen_for = ai.since_seen;
    ai.sees = can_see(t, &ai, world, &cx.sense, at);
    if ai.sees {
        if !was_seeing && unseen_for > at.reacquire_time {
            ai.aim_time = 0.0;
        }
        ai.since_seen = 0.0;
        ai.last_seen = Some(cx.sense.chest);
        ai.since_stimulus = 0.0;
    } else {
        ai.since_seen = (ai.since_seen + dt).min(1e6);
    }
    let ear = t.pos + Vec3::Y * at.eye_height;
    for n in cx.noises {
        if n.source == Some(t.id) {
            continue;
        }
        if n.pos.distance(ear) <= class.hearing_radius[n.group.min(2)] {
            ai.news = Some(n.interest);
        }
    }
    let threat = ai.threat.take();
    let news = ai.news.take().or(threat);

    // --- decisions ---
    if ai.sees {
        if matches!(ai.mode, Mode::Idle | Mode::Alerted { .. } | Mode::Search { .. }) {
            ai.mode = Mode::Combat;
        }
        if !ai.alerted {
            cx.alerts.push((t.id, t.pos, cx.sense.chest));
        }
        announce(t, &mut ai, cx);
    } else if let Some(p) = news {
        ai.since_stimulus = 0.0;
        if ai.last_seen.is_none() || !matches!(ai.mode, Mode::Combat | Mode::InCover { .. }) {
            ai.last_seen = Some(p);
        }
        match &mut ai.mode {
            Mode::Idle => ai.mode = Mode::Alerted { look_at: p, t: 0.0 },
            Mode::Alerted { look_at, .. } => *look_at = p,
            Mode::Search { to, arrived, t } => {
                *to = p;
                *arrived = false;
                *t = 0.0;
            }
            _ => {}
        }
        announce(t, &mut ai, cx);
    }
    if let Some(from) = threat
        && ai.cover_cooldown <= 0.0
        && !matches!(ai.mode, Mode::TakeCover { .. } | Mode::InCover { .. })
    {
        ai.cover_cooldown = at.cover_cooldown;
        let hide_from = if ai.sees { cx.sense.chest } else { from };
        if let Some(c) = find_cover(t, world, hide_from, cx) {
            ai.mode = Mode::TakeCover { to: c, t: 0.0 };
            ai.strafe_left = 0.0;
        }
    }

    // --- the mode ---
    let mut want = Vec3::ZERO;
    let mut face: Option<Vec3> = None;
    let mut may_fire = false;
    let mut crouch = false;
    let move_to = |pos: Vec3, to: Vec3, speed: f32| {
        let d = (to - pos).horizontal();
        let l = d.length();
        if l < 1e-4 { Vec3::ZERO } else { d / l * speed.min(l / dt.max(1e-4)) }
    };
    match ai.mode {
        Mode::Idle => {}
        Mode::Alerted { look_at, t: tt } => {
            face = Some(look_at);
            let tt = tt + dt;
            ai.mode = if tt >= at.alert_look_time {
                Mode::Search { to: look_at, t: 0.0, arrived: false }
            } else {
                Mode::Alerted { look_at, t: tt }
            };
        }
        Mode::Combat => {
            face = ai.last_seen;
            may_fire = true;
            if at.strafe_time_max > 0.0 {
                ai.next_strafe -= dt;
                if ai.strafe_left > 0.0 {
                    ai.strafe_left -= dt;
                    let side = yaw_right(t.yaw) * ai.strafe_dir;
                    let probe = t.pos + Vec3::Y * 0.5;
                    if world.raycast(probe, side, 1.0).is_some() {
                        ai.strafe_dir = -ai.strafe_dir;
                    }
                    want = yaw_right(t.yaw) * (ai.strafe_dir * ai.speeds.strafe);
                } else if ai.next_strafe <= 0.0 {
                    ai.strafe_dir = if cx.rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
                    ai.strafe_left = rand_range(cx.rng, at.strafe_time_min, at.strafe_time_max);
                    ai.next_strafe = ai.strafe_left
                        + rand_range(cx.rng, at.strafe_interval_min, at.strafe_interval_max);
                }
            }
            if !ai.sees && ai.since_seen > at.lose_sight_time {
                let to = ai.last_seen.unwrap_or(t.pos);
                ai.mode = Mode::Search { to, t: 0.0, arrived: false };
            }
        }
        Mode::TakeCover { to, t: tt } => {
            want = move_to(t.pos, to, ai.speeds.run);
            let tt = tt + dt;
            let arrived = (to - t.pos).horizontal().length() < 0.15;
            // Crouch on the tick of arrival: the next sight test then looks from the crouched
            // eye, so cover that hides a crouched gunman is not mistaken for flanked.
            crouch |= arrived;
            ai.mode = if arrived || tt > 8.0 {
                Mode::InCover { t: 0.0, peeking: false, unseen_peeks: 0 }
            } else {
                Mode::TakeCover { to, t: tt }
            };
        }
        Mode::InCover { t: tt, peeking, unseen_peeks } => {
            face = ai.last_seen;
            let tt = tt + dt;
            if peeking {
                may_fire = true;
                if tt >= at.peek_time {
                    let unseen = unseen_peeks + u32::from(ai.since_seen > at.peek_time - 1e-3);
                    ai.mode = if unseen >= at.max_unseen_peeks {
                        let to = ai.last_seen.unwrap_or(t.pos);
                        Mode::Search { to, t: 0.0, arrived: false }
                    } else {
                        Mode::InCover { t: 0.0, peeking: false, unseen_peeks: unseen }
                    };
                } else {
                    ai.mode = Mode::InCover { t: tt, peeking, unseen_peeks };
                }
            } else {
                crouch = true;
                if ai.sees {
                    // Flanked: the cover no longer hides it. Stand and fight.
                    ai.mode = Mode::Combat;
                    crouch = false;
                } else if tt >= at.hide_time {
                    ai.mode = Mode::InCover { t: 0.0, peeking: true, unseen_peeks };
                } else {
                    ai.mode = Mode::InCover { t: tt, peeking, unseen_peeks };
                }
            }
        }
        Mode::Search { to, t: tt, arrived } => {
            if arrived {
                if ai.since_stimulus > class.idle_timeout {
                    ai.mode = Mode::Idle;
                    ai.alerted = false;
                } else {
                    ai.mode = Mode::Search { to, t: tt + dt, arrived };
                }
            } else {
                want = move_to(t.pos, to, ai.speeds.run);
                let tt = tt + dt;
                let done = (to - t.pos).horizontal().length() < 0.6 || tt > 10.0;
                ai.mode = Mode::Search { to, t: if done { 0.0 } else { tt }, arrived: done };
            }
        }
    }
    if matches!(ai.mode, Mode::Alerted { .. }) && ai.since_stimulus > class.idle_timeout {
        ai.mode = Mode::Idle;
        ai.alerted = false;
    }
    if reacting {
        want = Vec3::ZERO;
        may_fire = false;
        crouch = ai.crouched;
    }
    ai.crouched = crouch;
    t.stance = stance(&ai, at);

    // --- turning and aiming ---
    let moving_dir = want.horizontal();
    let face_yaw = match face {
        Some(p) if (p - t.pos).horizontal().length() > 0.05 => Some(dir_yaw(p - t.pos)),
        _ if moving_dir.length() > 0.1 => Some(dir_yaw(moving_dir)),
        _ => None,
    };
    if let Some(y) = face_yaw
        && !matches!(t.state, TargetState::Stagger { .. })
    {
        t.yaw = approach_angle(t.yaw, y, class.head_turning_speed.to_radians() * dt);
    }
    let gun = muzzle(t, &ai, tu, Hand::Right);
    ai.gun = gun;
    let goal = if ai.sees {
        cx.sense.chest - gun
    } else {
        match face {
            Some(p) => p - gun,
            None => yaw_dir(t.yaw),
        }
    }
    .normalize_or_zero();
    if goal.length_sq() > 0.5 {
        ai.aim_dir = turn_towards(ai.aim_dir, goal, class.aiming_speed.to_radians() * dt);
    }
    if ai.sees {
        ai.aim_error = angle_between(ai.aim_dir, cx.sense.chest - gun);
        ai.aim_time += dt;
    }

    // --- shooting ---
    if may_fire && ai.sees && !reacting && ai.weapon.reload.is_none() {
        if ai.weapon.mag == 0 {
            ai.weapon.start_reload(def);
        } else if ai.cooldown <= 0.0
            && ai.aim_error <= class.trigger_happiness_angle.to_radians()
            && !ally_in_the_way(t.id, gun, cx.sense.chest, cx.allies, &cx.data.tuning)
        {
            fire(t, &mut ai, def, cx);
        }
    }
    if ai.weapon.mag == 0 && ai.weapon.reload.is_none() {
        ai.weapon.start_reload(def);
    }

    // --- moving ---
    let accel = cx.data.tuning.accel;
    ai.vel = approach_vec(ai.vel, want, accel * dt);
    move_body(t, &mut ai, world, tu, dt);

    // --- animation clock ---
    let (slot, _) = base_slot(t, &ai);
    if slot == ai.anim_slot {
        ai.anim_t += dt;
    } else {
        ai.anim_slot = slot;
        ai.anim_t = 0.0;
    }
    t.ai = Some(ai);
}

/// Does another enemy stand within `ally_clearance` of the line from `gun` to `goal`?
/// ASSUMPTION: MP2's gunmen rarely shoot each other in the back; payne's hold their fire instead.
fn ally_in_the_way(
    me: TargetId,
    gun: Vec3,
    goal: Vec3,
    allies: &[(TargetId, Vec3)],
    tu: &AiTuning,
) -> bool {
    let reach = (goal - gun).length();
    allies.iter().any(|&(id, feet)| {
        if id == me {
            return false;
        }
        // The ally's body as a vertical segment, sampled at hip, chest and head height.
        [0.9, 1.3, 1.65].iter().any(|h| {
            let p = feet + Vec3::Y * *h;
            let c = crate::world::closest_on_segment(gun, goal, p);
            (c - p).length() < tu.ally_clearance && (c - gun).length() < reach - 0.3
        })
    })
}

fn fire(t: &Target, ai: &mut Ai, def: &WeaponDef, cx: &mut EnemyCtx) {
    let hand = ai.weapon.consume_shot(def);
    ai.cooldown = def.fire_interval() / ai.skin.shooting_frequency_multiplier.max(1e-3);
    let origin = muzzle(t, ai, cx.tuning, hand);
    let err = (ai.inaccuracy(cx.difficulty) / 25.0).atan();
    let dir = cx.rng.cone(ai.aim_dir, err);
    for _ in 0..def.pellets.max(1) {
        let d = cx.rng.scatter(dir, def.spread_base);
        let speed = def.enemy_projectile_speed
            * (1.0 + (cx.rng.next_f32() * 2.0 - 1.0) * def.enemy_projectile_speed_random);
        cx.shots.push(Projectile {
            pos: origin,
            prev: origin,
            dir: d,
            speed,
            damage: def.damage,
            impulse: def.impulse,
            weapon: ai.weapon.def,
            travelled: 0.0,
            range: def.range,
            age: 0.0,
            owner: Some(t.id),
        });
    }
    cx.events.push(Event::EnemyShot {
        enemy: t.id,
        origin,
        dir,
    });
    cx.noises_out.push(Noise {
        pos: origin,
        group: 0,
        interest: ai.last_seen.unwrap_or(origin),
        source: Some(t.id),
    });
}

/// Move with the world like the player does: short sub-steps pushed out of walls, then follow the
/// ground (step up / snap down with the player's limits) or fall.
fn move_body<W: World + ?Sized>(t: &mut Target, ai: &mut Ai, world: &W, tu: &Tuning, dt: f32) {
    let r = ai.skin.capsule_radius;
    let top = ai.skin.capsule_top * t.stance;
    let delta = ai.vel * dt;
    let n = (delta.length() / (r * 0.5)).ceil().max(1.0) as usize;
    let mut total = Vec3::ZERO;
    for _ in 0..n {
        t.pos += delta / n as f32;
        let lo = t.pos.y + tu.step_up + r;
        let hi = (t.pos.y + top - r).max(lo);
        let mut push = world.push_out(
            Vec3::new(t.pos.x, lo, t.pos.z),
            Vec3::new(t.pos.x, hi, t.pos.z),
            r,
        );
        push.y = 0.0;
        t.pos += push;
        total += push;
    }
    let nrm = total.normalize_or_zero();
    let vn = ai.vel.dot(nrm);
    if vn < 0.0 {
        ai.vel -= nrm * vn;
    }
    if ai.airborne {
        let old_y = t.pos.y;
        let vy0 = ai.vy;
        ai.vy -= tu.gravity * dt;
        t.pos.y += 0.5 * (vy0 + ai.vy) * dt;
        if let Some(g) = world.ground(t.pos.x, t.pos.z, t.pos.y, old_y + 0.01) {
            t.pos.y = g.height;
            ai.vy = 0.0;
            ai.airborne = false;
        }
    } else {
        match world.ground(t.pos.x, t.pos.z, t.pos.y - tu.snap_down, t.pos.y + tu.step_up) {
            Some(g) => t.pos.y = g.height,
            None => {
                ai.airborne = true;
                ai.vy = 0.0;
            }
        }
    }
}

// --- animation ----------------------------------------------------------------------------------

/// What an enemy's skeleton should play. Look the slot up with [`clip_candidates`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyAnim {
    pub style: Style,
    /// Slot without style prefix, e.g. `RUNFORWARD`, `SMALLDAMAGE`.
    pub slot: &'static str,
    /// Seconds into the clip (world time; restarts when the slot changes).
    pub t: f32,
    pub looping: bool,
    /// World point the gun / upper body aims at, when aiming.
    pub aim: Option<Vec3>,
}

const RUN8: [&str; 8] = [
    "RUNFORWARD",
    "RUNFORWARDRIGHT",
    "RUNRIGHT",
    "RUNBACKWARDRIGHT",
    "RUNBACKWARD",
    "RUNBACKWARDLEFT",
    "RUNLEFT",
    "RUNFORWARDLEFT",
];
const WALK8: [&str; 8] = [
    "WALKFORWARD",
    "WALKFORWARDRIGHT",
    "WALKRIGHT",
    "WALKBACKWARDRIGHT",
    "WALKBACKWARD",
    "WALKBACKWARDLEFT",
    "WALKLEFT",
    "WALKFORWARDLEFT",
];

/// Slot and looping for the body's current state (hit reactions come from the target state).
fn base_slot(t: &Target, ai: &Ai) -> (&'static str, bool) {
    match t.state {
        TargetState::Flinch { .. } => return ("SMALLDAMAGE", false),
        TargetState::Stagger { .. } => return ("LARGEDAMAGE", false),
        _ => {}
    }
    let v = ai.vel.horizontal();
    let speed = v.length();
    if ai.crouched {
        return ("CROUCH", true);
    }
    if speed > 0.3 {
        let x = v.dot(yaw_right(t.yaw));
        let y = v.dot(yaw_dir(t.yaw));
        let a = x.atan2(y); // 0 forward, +right
        let i = ((a / (PI / 4.0)).round() as i32).rem_euclid(8) as usize;
        let walk = speed < (ai.speeds.walk + ai.speeds.run.min(ai.speeds.strafe)) * 0.5;
        return (if walk { WALK8[i] } else { RUN8[i] }, true);
    }
    if let Mode::Search { arrived: true, .. } = ai.mode {
        return ("CONFUSED", false);
    }
    ("STAND", true)
}

impl Target {
    /// What this enemy's skeleton should play; None for a plain target or a dead enemy (a normal
    /// MP2 death has no clip: the body goes straight to the ragdoll corpse).
    pub fn enemy_anim(&self) -> Option<EnemyAnim> {
        let ai = self.ai.as_ref()?;
        if !self.alive() {
            return None;
        }
        let (slot, looping) = base_slot(self, ai);
        let t = match self.state {
            TargetState::Flinch { t, .. } | TargetState::Stagger { t, .. } => t,
            _ if slot == ai.anim_slot => ai.anim_t,
            _ => 0.0,
        };
        let aiming = matches!(ai.mode, Mode::Combat | Mode::InCover { peeking: true, .. })
            || (ai.sees && !matches!(ai.mode, Mode::TakeCover { .. }));
        // A point 10 m down the gun from the muzzle.
        let aim = aiming.then(|| ai.gun + ai.aim_dir * 10.0);
        Some(EnemyAnim {
            style: ai.skin.style,
            slot,
            t,
            looping,
            aim,
        })
    }
}

/// Does a segment pass within `r` of point `p`? (near-miss test)
pub(crate) fn segment_near(a: Vec3, b: Vec3, p: Vec3, r: f32) -> bool {
    let c = crate::world::closest_on_segment(a, b, p);
    c.distance(p) <= r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_follow_the_style_order() {
        assert_eq!(
            clip_candidates(Style::Sharp, "RUNFORWARD"),
            ["SHARP/RUNFORWARD", "ENEMY/RUNFORWARD", "RUNFORWARD"].map(String::from)
        );
        assert_eq!(clip_candidates(Style::Plain, "STAND")[0], "ENEMY/STAND");
        assert_eq!(Style::parse("none"), Style::Plain);
        assert_eq!(Style::parse("sharp").name(), "SHARP");
    }

    #[test]
    fn turn_towards_is_limited() {
        let a = Vec3::new(0.0, 0.0, -1.0);
        let b = Vec3::X;
        let c = turn_towards(a, b, 0.5);
        assert!((angle_between(a, c) - 0.5).abs() < 1e-4);
        assert_eq!(turn_towards(a, b, 2.0), b);
        let back = turn_towards(a, -a, 0.3);
        assert!((angle_between(a, back) - 0.3).abs() < 1e-3);
    }

    #[test]
    fn loads_classes_skins_and_speeds() {
        let f = DataFile::parse(
            "[enemyclass C04]\nhealth = 60\ntrigger_happiness_angle = 20\nhearing_radius = 900 4 1\n\
             [enemyskin C04_BlackOps_A]\nclass = C04\nstyle = SHARP\ncapsule_radius = 0.3\n\
             [enemyskin Enemy_Boss]\nclass = C04\nstable_inaccuracy = 0.4\nstyle = none\n\
             [anim SHARP/RUNFORWARD]\nroot = 0 1 2\nfps = 2\n\
             [anim ENEMY/WALKFORWARD]\nroot = 0 0.04 0.08\n",
        )
        .unwrap();
        let d = EnemyData::from_file(&f);
        let s = d.skin("c04_blackops_a");
        assert_eq!((s.class.health, s.class.trigger_happiness_angle), (60.0, 20.0));
        assert_eq!(s.class.hearing_radius, [900.0, 4.0, 1.0]);
        assert_eq!((s.style, s.capsule_radius), (Style::Sharp, 0.3));
        assert_eq!(d.skin("Enemy_Boss").class.stable_inaccuracy, 0.4);
        let unknown = d.skin("Nobody");
        assert_eq!(unknown.class, EnemyClass::default());
        assert_eq!(d.clip_speed(Style::Sharp, "RUNFORWARD"), Some(2.0));
        // Walk curves without fps assume 30 fps: 0.08 m over 2 frames.
        assert!((d.clip_speed(Style::Normal, "WALKFORWARD").unwrap() - 1.2).abs() < 1e-4);
        assert_eq!(d.clip_speed(Style::Plain, "RUNFORWARD"), None);
    }
}
