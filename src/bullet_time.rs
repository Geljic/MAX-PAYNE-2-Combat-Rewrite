//! Bullet time: the hourglass meter, its fade, kill "efficiency", and the game-speed channel
//! that animations drive.
//!
//! Implements MP2's `SlowMotion_Update` as written up in `docs/behaviour/bullet-time.md`
//! (read from MaxPayne2.exe in Ghidra). Every timer runs on real, unscaled time. Constants are in
//! [`Tuning`] under their `[slowmotion]` names.
//!
//! Two things slow the world:
//! - **bullet time** (the toggle): `world = blend·lerp(0.6, 0.2, eff) + (1 − blend)` and
//!   `player = world · (blend·lerp(1, 3, eff) + (1 − blend))`;
//! - **game speed** set by animation messages (`Gm_ChangeGameSpeed(speed, blend)`), which is
//!   how the shootdodge and dodges slow time. It blends linearly to its target.
//!
//! How the two combine is not known yet; we take the slower of the two (ESTIMATE, see
//! [`BulletTime::world_time_scale`]).

use crate::math::approach;
use crate::tuning::Tuning;

#[derive(Clone, Debug, PartialEq)]
pub struct BulletTime {
    /// The hourglass, 0..1.
    pub meter: f32,
    /// Bullet time switched on.
    pub active: bool,
    /// How far bullet time has faded in, 0..1.
    pub blend: f32,
    /// Kill "adrenaline", 0..1 (MP2's hourglass turns yellow).
    pub efficiency: f32,
    /// While true (set by animation messages) the toggle is ignored.
    pub blocked: bool,
    /// Game speed requested by animation messages, and where it is blending to.
    pub game_speed: f32,
    game_speed_target: f32,
    game_speed_rate: f32,
    /// Efficiency blend after a kill: (from, to, elapsed).
    eff_blend: Option<(f32, f32, f32)>,
    /// Seconds since the efficiency blend finished (decay starts after the hold).
    eff_hold: f32,
}

impl Default for BulletTime {
    fn default() -> Self {
        BulletTime {
            meter: 1.0,
            active: false,
            blend: 0.0,
            efficiency: 0.0,
            blocked: false,
            game_speed: 1.0,
            game_speed_target: 1.0,
            game_speed_rate: 0.0,
            eff_blend: None,
            eff_hold: 0.0,
        }
    }
}

/// What a step of bullet time changed, for events.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BulletTimeChange {
    pub started: bool,
    pub ended: bool,
    /// Ended because the meter ran out.
    pub ran_out: bool,
    /// Tried to start with too little meter (MP2 plays CantSwitchOnMessages).
    pub refused: bool,
}

impl BulletTime {
    /// Toggle as from the bullet-time button.
    pub fn toggle(&mut self, tu: &Tuning) -> BulletTimeChange {
        let mut c = BulletTimeChange::default();
        if self.blocked {
            return c;
        }
        if self.active {
            self.active = false;
            self.meter = (self.meter - tu.turn_off_penalty).max(0.0);
            c.ended = true;
        } else if self.meter > tu.threshold {
            self.active = true;
            c.started = true;
        } else {
            c.refused = true;
        }
        c
    }

    /// Switch on or off regardless of the button (`Gm_EnableBulletTime`).
    pub fn set_active(&mut self, on: bool) -> BulletTimeChange {
        let mut c = BulletTimeChange::default();
        if on != self.active {
            self.active = on;
            c.started = on;
            c.ended = !on;
        }
        c
    }

    /// May the player shootdodge in slow motion now? Pays `ShootDodgePenalty` outside bullet time.
    pub fn begin_shootdodge(&mut self, tu: &Tuning) -> bool {
        if self.meter <= tu.shoot_dodge_threshold {
            return false;
        }
        if !self.active {
            self.meter = (self.meter - tu.shoot_dodge_penalty).max(0.0);
        }
        true
    }

    /// `Gm_ChangeGameSpeed(speed, blend)`: blend to `speed` over `blend` real seconds.
    pub fn change_game_speed(&mut self, speed: f32, blend: f32) {
        self.game_speed_target = speed;
        if blend <= 0.0 {
            self.game_speed = speed;
            self.game_speed_rate = 0.0;
        } else {
            self.game_speed_rate = (speed - self.game_speed).abs() / blend;
        }
    }

    /// A kill: meter bonus `slowmo_bonus` (the caller passes the victim's
    /// `SlowMotionBonusForKiller` × the difficulty's `BulletTimeGainedPerKillMultiplier`), and
    /// efficiency blends up by `EfficiencyIncreasePerKill`.
    pub fn on_kill(&mut self, slowmo_bonus: f32, tu: &Tuning) {
        self.meter = (self.meter + slowmo_bonus).min(1.0);
        let to = (self.efficiency + tu.efficiency_increase_per_kill).min(1.0);
        self.eff_blend = Some((self.efficiency, to, 0.0));
        self.eff_hold = 0.0;
    }

    /// Advance by `dt_real` (unscaled) seconds.
    pub fn update(&mut self, dt_real: f32, tu: &Tuning) -> BulletTimeChange {
        let mut c = BulletTimeChange::default();
        if self.active {
            self.blend = (self.blend + dt_real / tu.in_blend_time.max(1e-4)).min(1.0);
            self.meter -= dt_real / tu.slowmotion_time.max(1e-4);
            if self.meter <= 0.0 {
                self.meter = 0.0;
                self.active = false;
                c.ended = true;
                c.ran_out = true;
            }
        } else {
            self.blend = (self.blend - dt_real / tu.out_blend_time.max(1e-4)).max(0.0);
            self.meter = (self.meter + dt_real / tu.regeneration_time.max(1e-4)).min(1.0);
        }

        // Efficiency: blend up after a kill, hold, then decay.
        if let Some((from, to, t)) = self.eff_blend.as_mut() {
            *t += dt_real;
            let u = (*t / tu.efficiency_increase_blend_time.max(1e-4)).min(1.0);
            self.efficiency = *from + (*to - *from) * u;
            if u >= 1.0 {
                self.eff_blend = None;
                self.eff_hold = 0.0;
            }
        } else if self.efficiency > 0.0 {
            self.eff_hold += dt_real;
            if self.eff_hold > tu.efficiency_decrease_delay_after_kill {
                self.efficiency =
                    (self.efficiency - tu.efficiency_decrease_speed * dt_real).max(0.0);
            }
        }

        self.game_speed = approach(
            self.game_speed,
            self.game_speed_target,
            self.game_speed_rate * dt_real,
        );
        c
    }

    /// World speed from bullet time alone.
    pub fn bullet_time_world_speed(&self, tu: &Tuning) -> f32 {
        let slow = lerp(
            tu.min_efficiency_game_speed_multiplier,
            tu.max_efficiency_game_speed_multiplier,
            self.efficiency,
        );
        self.blend * slow + (1.0 - self.blend)
    }

    /// Factor by which the player's clock runs faster than the world's.
    pub fn player_factor(&self, tu: &Tuning) -> f32 {
        self.blend
            * lerp(
                1.0,
                tu.max_efficiency_player_time_multiplier,
                self.efficiency,
            )
            + (1.0 - self.blend)
    }

    /// How fast the world runs (1 = normal). ESTIMATE: bullet time and animation game speed
    /// combine as the slower of the two.
    pub fn world_time_scale(&self, tu: &Tuning) -> f32 {
        self.bullet_time_world_speed(tu).min(self.game_speed)
    }

    /// How fast the player's body, animations and weapons run. Aiming is never scaled.
    pub fn player_time_scale(&self, tu: &Tuning) -> f32 {
        self.world_time_scale(tu) * self.player_factor(tu)
    }

    /// Kill adrenaline shown on the HUD (the yellow hourglass).
    pub fn adrenaline(&self) -> f32 {
        self.efficiency
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
