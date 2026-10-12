//! Every feel number for the player and bullet time in one place.
//!
//! Field names match the snake_case keys of the `[character ...]` and `[slowmotion]` sections of
//! `payne.data` (see [`crate::data`]), so game data overrides a default just by naming it. The
//! same names work in a hand-written `payne.tune` (`name = value` lines) for live tuning.
//!
//! Defaults: values copied from the retail MP2 database are labelled with their source file;
//! everything else is an ESTIMATE with a note on where the real value probably lives. The
//! defaults only matter until `payne.data` is loaded.

macro_rules! tuning {
    ($( $(#[doc = $doc:literal])* $name:ident : $val:expr ),* $(,)?) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct Tuning { $( $(#[doc = $doc])* pub $name: f32, )* }

        impl Default for Tuning {
            fn default() -> Self { Tuning { $( $name: $val, )* } }
        }

        impl Tuning {
            /// Set one value by name. Returns false for unknown names.
            pub fn set(&mut self, name: &str, v: f32) -> bool {
                match name { $( stringify!($name) => { self.$name = v; true } )* _ => false }
            }

            /// Read one value by name.
            pub fn get(&self, name: &str) -> Option<f32> {
                match name { $( stringify!($name) => Some(self.$name), )* _ => None }
            }

            /// All values as `name = value` lines, with their docs as comments.
            pub fn to_text(&self) -> String {
                let mut s = String::new();
                $(
                    $( s.push_str(&format!("#{}\n", $doc)); )*
                    s.push_str(&format!("{} = {}\n\n", stringify!($name), self.$name));
                )*
                s
            }
        }
    };
}

tuning! {
    // --- body (skins/MaxPayne.txt [Properties]) ---
    /// Capsule top above the feet (m). MaxPayne.txt CapsuleTop.
    capsule_top: 1.95,
    /// Capsule radius (m). MaxPayne.txt CapsuleRadius.
    capsule_radius: 0.48,
    /// Player health. MaxPayne.txt MaximumHealth.
    max_health: 250.0,
    /// Highest step walked up without jumping (m). ESTIMATE: MP2 character code (X_GameObjectsMFC).
    step_up: 0.35,
    /// How far the feet follow the ground down without falling (m). ESTIMATE.
    snap_down: 0.4,
    /// Capsule top while crouched (m). Default is an ESTIMATE; `payne-extract measure` supplies it from
    /// the head bone in Crouch.kf2 (measured: CROUCH, see docs/behaviour/measured.md).
    crouch_top: 1.2,
    /// Capsule top while lying down after a shootdodge (m). Default is an ESTIMATE; `payne-extract
    /// measure` supplies it from the final pose of the four ShootDodge clips (measured, see
    /// docs/behaviour/measured.md).
    prone_top: 0.6,

    // --- ground movement ---
    /// Forward run speed (m/s). Root motion of Run_Forward_mov.kf2: 4.20 m in 40 frames at 50 fps.
    run_speed: 5.25,
    /// Sideways run speed (m/s). Measured: Run_Left/Right_mov.kf2 root motion (4.2 m per 0.8 s).
    strafe_speed: 4.6,
    /// Backward run speed (m/s). Measured: Run_Backward_mov.kf2 root motion (2.5 m per 0.667 s).
    backpedal_speed: 4.0,
    /// Crouched move speed (m/s). ESTIMATE: the retail data has no crouch-walk clip to measure
    /// (CROUCH is a held pose); the real value is in the MP2 character code.
    crouch_speed: 2.0,
    /// Acceleration towards the wanted speed (m/s²). ESTIMATE: MP2 blends run cycles in quickly.
    accel: 30.0,
    /// Deceleration when stopping (m/s²). ESTIMATE.
    decel: 40.0,
    /// Stick deflection below this counts as no input.
    deadzone: 0.15,
    /// Body turn rate towards the aim yaw on foot (rad/s, real time). ESTIMATE: Max faces the aim
    /// almost instantly in MP2.
    body_turn_rate: 20.0,

    // --- aim ---
    /// Lowest aim pitch (rad). ESTIMATE: MP2 camera code.
    pitch_min: -1.2,
    /// Highest aim pitch (rad). ESTIMATE.
    pitch_max: 1.2,
    /// The aim point is kept at least this far from the character along the camera ray (m).
    /// X_Character::update2DAiming (read in Ghidra).
    aim_min_distance: 1.5,
    /// Crosshair ray length when it hits nothing (m). shooting.h CROSSHAIR_CASTLENGTH.
    aim_cast_length: 100.0,
    /// Muzzle height above the feet standing / crouched / lying (m). Measured: the `Bip01 R Gun` bone
    /// in SHOOTBERETTA, in CROUCH and in the last frame of SHOOTDODGEFORWARD (docs/behaviour/measured.md).
    muzzle_stand: 1.4,
    muzzle_crouch: 0.95,
    muzzle_prone: 0.4,
    /// Muzzle offset to the side of the body (m). Measured: `Bip01 R Gun` x in SHOOTBERETTA.
    muzzle_side: 0.22,

    // --- options ---
    /// 1 = a shootdodge slows the world on its own (MP2's behaviour, from the dive's
    /// Gm_ChangeGameSpeed frame messages); 0 = only bullet time (right mouse) slows the world.
    /// Payne option, default 1 (MP2's behaviour: the dive's slow motion lasts as long as the
    /// original's).
    shootdodge_slowmo: 1.0,
    /// 1 = shootdodge pressed in the air dives from there (the "jump dive"). Remedy defined
    /// HIGHSHOOTDODGE* animation slots but shipped none (they revert to POSE), so this is a payne
    /// option, default 1. The dive enters its clip at the `C_Jump` frame, keeps the vertical speed
    /// and flies at the clip's airborne speed.
    jump_dive: 1.0,
    /// Upward speed added when a jump dive starts (m/s). ESTIMATE / payne option: MP2 has no jump
    /// dive to measure; 0 keeps the fall purely ballistic.
    jump_dive_boost: 0.0,

    // --- air ---
    /// Gravity (m/s²). From `levels.txt` (`GRAVITY_VALUE`, -20 in every retail level), which the
    /// character controller integrates directly (X_PhysicalSimulationMFC); `[level]` in payne.data.
    gravity: 20.0,
    /// Multiplier on C_Jump(v) arguments. 1.0 = the argument is the take-off speed in m/s.
    /// With MP2's gravity of 20, Jump_Up's C_Jump(7.5) peaks at 1.41 m.
    jump_scale: 1.0,
    /// Fallback take-off speed when the jump animation has no C_Jump message (m/s). ESTIMATE.
    jump_speed: 4.5,
    /// Horizontal steering speed in the air (m/s). MaxPayne.txt AirborneSpeed.
    airborne_speed: 2.5,
    /// Steering acceleration in the air (m/s²). ESTIMATE.
    air_accel: 6.0,
    /// Landing recovery after a normal fall (s). ESTIMATE: no soft-landing clip exists (Land.kf2 is
    /// the hard landing, see `hard_land_time`).
    land_time: 0.2,
    /// Falls from at least this high are hard landings (m). ESTIMATE.
    hard_land_height: 3.0,
    /// Hard landing recovery (s). Measured: length of LAND (Land.kf2), which tags `dummy_land_hard`.
    hard_land_time: 0.6,

    // --- shootdodge / dodge (frames are in the move's own animation frames) ---
    /// The dodge roll can still turn into a shootdodge before this frame. MaxPayne.txt CanShootDodgeFrame.
    can_shootdodge_frame: 13.0,
    /// From this shootdodge frame on, move input gets up early. MaxPayne.txt CanAbortShootDodgeFrame.
    can_abort_shootdodge_frame: 49.0,
    /// Slide deceleration once lying down (m/s²). ESTIMATE. With a measured root curve the dive's
    /// own animation slides to a stop before Prone starts, so this only acts without game data
    /// (constant-speed fallback) or on speed left over from a fall.
    prone_slide_decel: 12.0,
    /// Lying down, the upper body twists this far from the body before the body turns (rad). ESTIMATE.
    prone_twist: 1.6,
    /// Lying down, the body turns at this rate to follow the aim (rad/s, real time). ESTIMATE.
    prone_turn_rate: 6.0,
    /// Below this roll speed (m/s) crouch toggles crouching instead of rolling. ESTIMATE.
    roll_min_speed: 1.0,

    // --- player health ---
    /// Painkillers carried at the start. ESTIMATE: Max's skin starts with one
    /// (`c_pickupweapon painkiller` in MaxPayne.txt); levels hand out more.
    painkillers_start: 3.0,
    /// Most painkillers carried. ESTIMATE: the MP2 inventory limit (game code).
    painkillers_max: 8.0,
    /// Radius of the capsule enemy bullets hit (m). ESTIMATE: MP2 tests Max's target bones
    /// (Head / Torso / ArmsAndLegs in MaxPayne.txt); one capsule thinner than the 0.48 m movement
    /// capsule stands in for them.
    player_hit_radius: 0.35,
    /// Length of the lying body's hit capsule during a dive, prone and get-up (m). ESTIMATE.
    prone_hit_length: 1.7,

    // --- player hit reactions ---
    /// Flinch duration (s). Measured: length of CHARANIM_SMALLDAMAGE (GetHit_Small.kf2).
    flinch_time: 0.3,

    // --- weapons ---
    /// Time to switch weapons (s). Measured: length of GETWEAPON (PutGetGun_Split.kf2), the draw;
    /// HOLSTERWEAPON (PutGetGun.kf2) adds 0.333 s if the sim ever plays it first.
    switch_time: 0.35,
    /// Recoil kick recovery (rad/s). ESTIMATE.
    recoil_recovery: 0.6,
    /// Spread multipliers by stance. MP2's projectile launch applies none (only the weapon's
    /// Accuracy), so all default to 1; payne options for games that want them.
    spread_moving: 1.0,
    spread_crouch: 1.0,
    spread_air: 1.0,
    spread_prone: 1.0,

    // --- bullet time (globalconfig.txt [slowmotion]; fractions, so 7% = 0.07) ---
    /// Seconds a full meter lasts. SlowMotionTime.
    slowmotion_time: 15.0,
    /// Seconds to recharge 0 → 1 while off. RegenerationTime.
    regeneration_time: 90.0,
    /// Meter needed to switch bullet time on. Threshold.
    threshold: 0.07,
    /// Meter lost when switching off by hand. TurnOffPenalty.
    turn_off_penalty: 0.0,
    /// Meter lost by a shootdodge outside bullet time. ShootDodgePenalty.
    shoot_dodge_penalty: 0.0,
    /// Meter needed for a slow-motion shootdodge. ShootDodgeThreshold.
    shoot_dodge_threshold: 0.0001,
    /// Efficiency gained per kill. EfficiencyIncreasePerKill.
    efficiency_increase_per_kill: 0.4,
    /// Efficiency holds this long after a kill (s). EfficiencyDecreaseDelayAfterKill.
    efficiency_decrease_delay_after_kill: 5.0,
    /// Efficiency lost per second after the hold. EfficiencyDecreaseSpeed.
    efficiency_decrease_speed: 0.15,
    /// World speed in full bullet time at zero efficiency. MinEfficiencyGameSpeedMultiplier.
    min_efficiency_game_speed_multiplier: 0.6,
    /// World speed in full bullet time at full efficiency. MaxEfficiencyGameSpeedMultiplier.
    max_efficiency_game_speed_multiplier: 0.2,
    /// Player clock factor at full efficiency (1.0 at zero). MaxEfficiencyPlayerTimeMultiplier.
    max_efficiency_player_time_multiplier: 3.0,
    /// Fade-in time (s). InBlendTime.
    in_blend_time: 0.3,
    /// Fade-out time (s). OutBlendTime.
    out_blend_time: 1.5,
    /// Efficiency rises to its new value over this time (s). EfficiencyIncreaseBlendTime.
    efficiency_increase_blend_time: 0.2,

    // --- ragdoll (ragdolls/highdetail.txt where noted) ---
    /// Constraint solver iterations per step. ESTIMATE.
    ragdoll_iterations: 12.0,
    /// Velocity damping (1/s). ESTIMATE.
    ragdoll_damping: 0.4,
    /// Bounce off surfaces. highdetail.txt Restitution (default 0.2).
    ragdoll_restitution: 0.2,
    /// Surface friction. highdetail.txt Friction (default 0.5).
    ragdoll_friction: 0.5,
    /// Multiplier on weapon impulses hitting a body. ESTIMATE: MP2 deaths are exaggerated.
    ragdoll_impulse_scale: 1.0,
    /// Falls asleep when every particle is slower than this for `ragdoll_sleep_time` (m/s). ESTIMATE.
    ragdoll_sleep_speed: 0.08,
    ragdoll_sleep_time: 0.5,

    // --- projectiles ---
    /// Projectiles are removed after flying this long in world time (s). ESTIMATE.
    projectile_lifetime: 4.0,
}

impl Tuning {
    /// Apply a `name = value` text (comments with #). Returns the names it did not know.
    pub fn apply_text(&mut self, text: &str) -> Vec<String> {
        let mut unknown = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match crate::data::parse_number(v) {
                Some(v) if self.set(k, v) => {}
                _ => unknown.push(k.to_string()),
            }
        }
        unknown
    }

    pub fn from_text(text: &str) -> (Tuning, Vec<String>) {
        let mut t = Tuning::default();
        let u = t.apply_text(text);
        (t, u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let t = Tuning {
            run_speed: 9.0,
            ..Default::default()
        };
        let (back, unknown) = Tuning::from_text(&t.to_text());
        assert!(unknown.is_empty());
        assert_eq!(back, t);
        let mut t = Tuning::default();
        assert_eq!(
            t.apply_text("threshold = 10%\nnope = 1"),
            vec!["nope".to_string()]
        );
        assert!((t.threshold - 0.1).abs() < 1e-6);
    }
}
