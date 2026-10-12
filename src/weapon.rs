//! Weapons: static definitions ([`WeaponDef`]) and per-player state ([`WeaponState`]).
//!
//! The default roster takes clip size, pocket size, damage, penetration, shooting frequency,
//! slow-motion multiplier and spread from MP2's `database/shooting.h`, pellet counts from
//! `weapons/*.txt` and bullet speed from `BULLETVELOCITY_PLAYER`. Reload times and recoil are
//! ESTIMATEs (reload length lives in the weapon's reload animation). Shots scatter the way MP2's
//! projectile launch does (`Rng::scatter`); the `bloom` fields are tracked but not used for firing,
//! since MP2 has no sustained-fire bloom. `payne.data` `[weapon ...]` sections replace these at runtime.

use crate::math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireMode {
    /// One shot per trigger press.
    Semi,
    /// Fires while the trigger is held.
    Auto,
}

/// Which upper-body animation family a weapon uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeaponClass {
    Pistol,
    DualPistol,
    Shotgun,
    Smg,
    DualSmg,
    Rifle,
}

impl WeaponClass {
    pub fn name(self) -> &'static str {
        match self {
            WeaponClass::Pistol => "pistol",
            WeaponClass::DualPistol => "dual_pistol",
            WeaponClass::Shotgun => "shotgun",
            WeaponClass::Smg => "smg",
            WeaponClass::DualSmg => "dual_smg",
            WeaponClass::Rifle => "rifle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Right,
    Left,
}

/// MP2 spread units (`*_BULLETSPREAD`, the projectile's `Accuracy`) to the scatter maximum: the game
/// adds a random unit vector times `uniform(0,1) × Accuracy / 2500` to the shot direction
/// (X_GameObjectsMFC projectile launch). See `Rng::scatter`.
pub fn spread_units_to_rad(units: f32) -> f32 {
    units / 2500.0
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponDef {
    pub name: String,
    pub class: WeaponClass,
    pub fire_mode: FireMode,
    /// Two guns, firing alternately from each hand.
    pub dual: bool,
    pub two_handed: bool,
    pub shootdodge_allowed: bool,
    /// Shots per second. shooting.h *_SHOOTINGFREQUENCY.
    pub shooting_frequency: f32,
    /// Damage per bullet / pellet. shooting.h *_BULLETDAMAGE.
    pub damage: f32,
    /// shooting.h *_BULLETPENETRATION. Stored for later: the test world has nothing to penetrate.
    pub penetration: f32,
    /// shooting.h *_SLOWMOTIONMULTIPLIER (the weapon's ShootingFrequencyMultiplier). Stored, not
    /// applied: how MP2 uses it in bullet time is not known yet.
    pub slowmo_multiplier: f32,
    /// Rounds per magazine. shooting.h *_CLIPSIZE.
    pub magazine: u32,
    /// Most spare rounds carried. shooting.h *_POCKETSIZE.
    pub pocket: u32,
    /// Rounds added per reload cycle (pump shotgun: 1). weapons/*.txt ReloadAmount.
    pub reload_amount: u32,
    /// Seconds per reload cycle. ESTIMATE: the weapon's reload animation length.
    pub reload_time: f32,
    /// Base cone half-angle (rad), from shooting.h *_BULLETSPREAD via [`spread_units_to_rad`].
    pub spread_base: f32,
    /// Most extra spread from sustained fire (rad). ESTIMATE.
    pub spread_bloom_max: f32,
    /// Extra spread per shot (rad). ESTIMATE.
    pub spread_per_shot: f32,
    /// Extra spread recovered per second (rad/s). ESTIMATE.
    pub spread_recovery: f32,
    /// Aim pitch kick per shot (rad). ESTIMATE.
    pub recoil: f32,
    /// Projectiles per shot (pump shotgun: 1 straight + 8 spread). weapons/*.txt P_CreateProjectile.
    pub pellets: u32,
    /// Bullet speed (m/s). shooting.h BULLETVELOCITY_PLAYER.
    pub projectile_speed: f32,
    /// Random fraction of the speed (±). shooting.h BULLETVELOCITY_PLAYERRANDOM.
    pub projectile_speed_random: f32,
    /// Enemy bullet speed (m/s). shooting.h BULLETVELOCITY_ENEMY, the `_Enemy` projectile's Speed.
    pub enemy_projectile_speed: f32,
    /// Random fraction of the enemy bullet speed (±). The `_Enemy` projectile's SpeedRandom.
    pub enemy_projectile_speed_random: f32,
    /// Push a bullet gives a body (N·s). ESTIMATE.
    pub impulse: f32,
    /// Bullets fly at most this far (m). ESTIMATE.
    pub range: f32,
}

impl WeaponDef {
    /// Seconds between shots.
    pub fn fire_interval(&self) -> f32 {
        if self.shooting_frequency > 0.0 {
            1.0 / self.shooting_frequency
        } else {
            1.0
        }
    }

    fn base(name: &str, class: WeaponClass) -> WeaponDef {
        WeaponDef {
            name: name.to_string(),
            class,
            fire_mode: FireMode::Semi,
            dual: false,
            two_handed: false,
            shootdodge_allowed: true,
            shooting_frequency: 4.0,
            damage: 17.0,
            penetration: 0.2,
            slowmo_multiplier: 0.25,
            magazine: 16,
            pocket: 160,
            reload_amount: 16,
            reload_time: 1.4,
            spread_base: spread_units_to_rad(100.0),
            spread_bloom_max: 0.03,
            spread_per_shot: 0.008,
            spread_recovery: 0.08,
            recoil: 0.015,
            pellets: 1,
            projectile_speed: 150.0,
            projectile_speed_random: 0.05,
            enemy_projectile_speed: 50.0,
            enemy_projectile_speed_random: 0.05,
            impulse: 10.0,
            range: 300.0,
        }
    }
}

/// MP2's milestone weapons. Indices are stable: 0 Beretta, 1 Dual Berettas, 2 Desert Eagle,
/// 3 Pump Shotgun, 4 Ingram (dual), 5 MP5, 6 Kalashnikov.
pub fn default_roster() -> Vec<WeaponDef> {
    use WeaponClass::*;
    let beretta = WeaponDef::base("Beretta", Pistol);
    let dual = WeaponDef {
        dual: true,
        magazine: 32,
        reload_amount: 32,
        reload_time: 1.8,
        spread_base: spread_units_to_rad(150.0),
        ..WeaponDef::base("BerettaDual", DualPistol)
    };
    let deagle = WeaponDef {
        shooting_frequency: 3.0,
        damage: 41.0,
        penetration: 0.3,
        magazine: 10,
        pocket: 100,
        reload_amount: 10,
        reload_time: 1.5,
        spread_base: spread_units_to_rad(25.0),
        spread_per_shot: 0.015,
        recoil: 0.05,
        impulse: 25.0,
        ..WeaponDef::base("DesertEagle", Pistol)
    };
    let pump = WeaponDef {
        two_handed: true,
        shooting_frequency: 1.5,
        damage: 9.0,
        penetration: 0.15,
        slowmo_multiplier: 0.2,
        magazine: 7,
        pocket: 140,
        reload_amount: 1,
        reload_time: 0.5,
        spread_base: spread_units_to_rad(300.0),
        spread_per_shot: 0.0,
        spread_bloom_max: 0.0,
        recoil: 0.06,
        pellets: 9,
        impulse: 6.0,
        range: 60.0,
        ..WeaponDef::base("PumpShotgun", Shotgun)
    };
    let ingram = WeaponDef {
        fire_mode: FireMode::Auto,
        dual: true,
        shooting_frequency: 9.0,
        damage: 12.0,
        penetration: 0.15,
        slowmo_multiplier: 0.0,
        magazine: 64,
        pocket: 320,
        reload_amount: 64,
        reload_time: 2.0,
        spread_base: spread_units_to_rad(375.0),
        spread_per_shot: 0.006,
        spread_bloom_max: 0.06,
        recoil: 0.008,
        impulse: 7.0,
        ..WeaponDef::base("IngramDual", DualSmg)
    };
    let mp5 = WeaponDef {
        fire_mode: FireMode::Auto,
        two_handed: true,
        shooting_frequency: 8.0,
        damage: 23.0,
        penetration: 0.25,
        slowmo_multiplier: 0.5,
        magazine: 30,
        pocket: 300,
        reload_amount: 30,
        reload_time: 1.8,
        spread_base: spread_units_to_rad(33.0),
        spread_per_shot: 0.006,
        spread_bloom_max: 0.05,
        recoil: 0.01,
        impulse: 9.0,
        ..WeaponDef::base("MP5", Smg)
    };
    let ak = WeaponDef {
        fire_mode: FireMode::Auto,
        two_handed: true,
        shooting_frequency: 7.0,
        damage: 31.0,
        penetration: 0.4,
        slowmo_multiplier: 0.6,
        magazine: 30,
        pocket: 300,
        reload_amount: 30,
        reload_time: 2.0,
        spread_base: spread_units_to_rad(75.0),
        spread_per_shot: 0.008,
        spread_bloom_max: 0.06,
        recoil: 0.018,
        impulse: 14.0,
        ..WeaponDef::base("Kalashnikov", Rifle)
    };
    vec![beretta, dual, deagle, pump, ingram, mp5, ak]
}

/// A request to spawn one projectile, produced when the player fires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub weapon: usize,
    pub origin: Vec3,
    pub dir: Vec3,
    pub speed: f32,
    pub damage: f32,
    pub impulse: f32,
    pub range: f32,
    pub hand: Hand,
}

/// One carried weapon.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponState {
    /// Index into the roster.
    pub def: usize,
    pub mag: u32,
    pub reserve: u32,
    /// Seconds until the next shot may fire (player time). May go slightly negative while the
    /// trigger is held so automatic fire keeps its exact rate at any tick rate.
    pub cooldown: f32,
    /// Seconds into the current reload cycle, if reloading.
    pub reload: Option<f32>,
    /// Extra spread from sustained fire (rad).
    pub bloom: f32,
    /// Player seconds since the last shot (bloom recovers once the gun rests).
    pub since_shot: f32,
    /// Hand of the next shot (dual weapons alternate).
    pub next_hand: Hand,
}

impl WeaponState {
    /// A full weapon with `reserve` spare rounds (clamped to the pocket size).
    pub fn new(def_index: usize, def: &WeaponDef, reserve: u32) -> Self {
        WeaponState {
            def: def_index,
            mag: def.magazine,
            reserve: reserve.min(def.pocket),
            cooldown: 0.0,
            reload: None,
            bloom: 0.0,
            since_shot: 1e6,
            next_hand: Hand::Right,
        }
    }

    /// Current cone half-angle before stance multipliers (rad).
    pub fn spread(&self, def: &WeaponDef) -> f32 {
        def.spread_base + self.bloom
    }

    pub fn can_reload(&self, def: &WeaponDef) -> bool {
        self.reload.is_none() && self.mag < def.magazine && self.reserve > 0
    }

    pub fn start_reload(&mut self, def: &WeaponDef) -> bool {
        if self.can_reload(def) {
            self.reload = Some(0.0);
            true
        } else {
            false
        }
    }

    pub fn cancel_reload(&mut self) -> bool {
        self.reload.take().is_some()
    }

    /// Move one reload cycle's rounds from the reserve into the magazine.
    pub fn load_rounds(&mut self, def: &WeaponDef) {
        // 0 means a full magazine, like MP2's ReloadAmount = -1.
        let per_cycle = if def.reload_amount == 0 { def.magazine } else { def.reload_amount };
        let n = per_cycle
            .min(def.magazine - self.mag.min(def.magazine))
            .min(self.reserve);
        self.mag += n;
        self.reserve -= n;
    }

    /// Advance timers by `dt` player seconds. Returns true when a reload finished this step.
    pub fn tick(&mut self, def: &WeaponDef, dt: f32) -> bool {
        self.cooldown -= dt;
        self.since_shot = (self.since_shot + dt).min(1e6);
        // Recovery waits until the gun has rested a little longer than its fire interval, so
        // sustained fire builds up spread.
        if self.since_shot > def.fire_interval() * 1.5 {
            self.bloom = (self.bloom - def.spread_recovery * dt).max(0.0);
        }
        if let Some(t) = self.reload.as_mut() {
            *t += dt;
            if *t >= def.reload_time {
                self.load_rounds(def);
                // Shell-by-shell weapons keep cycling until full or out of rounds.
                self.reload = (self.mag < def.magazine && self.reserve > 0).then_some(0.0);
                return self.reload.is_none();
            }
        }
        false
    }

    /// Spend one round and start the cooldown. Returns the firing hand.
    pub fn consume_shot(&mut self, def: &WeaponDef) -> Hand {
        let hand = self.next_hand;
        if def.dual {
            self.next_hand = if hand == Hand::Right {
                Hand::Left
            } else {
                Hand::Right
            };
        }
        self.mag -= 1;
        // Carry the overshoot (bounded) so held automatic fire averages exactly its frequency.
        self.cooldown = self.cooldown.max(-def.fire_interval()) + def.fire_interval();
        self.bloom = (self.bloom + def.spread_per_shot).min(def.spread_bloom_max);
        self.since_shot = 0.0;
        hand
    }
}
