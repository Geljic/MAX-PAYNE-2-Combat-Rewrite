//! The whole simulation behind one call: [`Sim::step`] advances the player, bullet time,
//! projectiles, targets and bodies by one 60 Hz tick and collects [`Event`]s.
//!
//! Clocks per tick (`DT` real seconds):
//! - bullet time and aiming: real `DT`;
//! - the player: `DT × player_time_scale`;
//! - projectiles, targets, enemies and ragdolls: `DT × world_time_scale`.
//!
//! Enemies ([`Sim::spawn_enemy`]) are targets with a brain ([`crate::enemy`]). Each tick, after the
//! player and the projectiles, they perceive (sight, the noises of the last tick), decide, aim,
//! shoot and move. Their bullets hit the player's [`Player::hit_capsule`], other enemies and the
//! level, never their shooter.

use crate::bullet_time::BulletTime;
use crate::data::GameData;
use crate::enemy::{self, Ai, Difficulty, EnemyCtx, EnemyData, Noise, Sense};
use crate::event::Event;
use crate::math::{Rng, Vec3, ray_capsule, ray_sphere};
use crate::moves::MoveSet;
use crate::player::{Input, Player, Pressed, State, StepCtx};
use crate::projectile::Projectile;
use crate::ragdoll::{Ragdoll, SkeletonDef};
use crate::target::{Target, TargetDef};
use crate::tuning::Tuning;
use crate::weapon::{WeaponDef, default_roster};
use crate::world::{HitZone, TargetId, World};

/// A dead body.
#[derive(Clone, Debug)]
pub struct Corpse {
    /// The target it was, or None for the player.
    pub owner: Option<TargetId>,
    pub ragdoll: Ragdoll,
}

#[derive(Clone, Debug)]
pub struct Sim {
    pub tuning: Tuning,
    /// The weapon roster; weapon indices in events point here.
    pub weapons: Vec<WeaponDef>,
    pub moves: MoveSet,
    pub skeleton: SkeletonDef,
    /// Definition given to targets spawned with [`Sim::spawn_target`].
    pub target_def: TargetDef,
    /// Enemy classes, skins, clip speeds and AI tuning.
    pub enemies: EnemyData,
    /// Difficulty: enemy inaccuracy, painkillers, kill bonus multiplier.
    pub difficulty: Difficulty,
    pub player: Player,
    pub targets: Vec<Target>,
    pub corpses: Vec<Corpse>,
    pub projectiles: Vec<Projectile>,
    pub bullet_time: BulletTime,
    /// Real and world seconds simulated.
    pub time: f64,
    pub world_time: f64,
    events: Vec<Event>,
    rng: Rng,
    prev_input: Input,
    next_id: u32,
    /// Noises made since the enemies last listened.
    noises: Vec<Noise>,
}

/// Nearest thing a ray met.
enum Struck {
    World(crate::world::RayHit),
    Target(usize, HitZone, Vec3, Vec3),
    Player,
    Corpse(usize, Vec3),
}

impl Sim {
    /// The fixed tick (s).
    pub const DT: f32 = 1.0 / 60.0;

    /// A sim with ESTIMATE defaults and the default roster; the player stands at `spawn`.
    pub fn new(spawn: Vec3, yaw: f32) -> Sim {
        Sim::with_data(spawn, yaw, GameData::default())
    }

    /// A sim using loaded game data (see [`GameData::load`]).
    pub fn with_data(spawn: Vec3, yaw: f32, data: GameData) -> Sim {
        let weapons = if data.weapons.is_empty() {
            default_roster()
        } else {
            data.weapons
        };
        let player = Player::new(spawn, yaw, &weapons, &data.tuning);
        Sim {
            tuning: data.tuning,
            weapons,
            moves: data.moves,
            skeleton: data.skeleton,
            target_def: data.target,
            enemies: data.enemies,
            difficulty: data.difficulty,
            player,
            targets: Vec::new(),
            corpses: Vec::new(),
            projectiles: Vec::new(),
            bullet_time: BulletTime::default(),
            time: 0.0,
            world_time: 0.0,
            events: Vec::new(),
            rng: Rng::new(0x5eed_1234),
            prev_input: Input::default(),
            next_id: 1,
            noises: Vec::new(),
        }
    }

    /// Advance one tick.
    pub fn step<W: World + ?Sized>(&mut self, world: &W, input: &Input) {
        let dt = Self::DT;
        let pressed = Pressed::between(&self.prev_input, input);

        if pressed.bullet_time && self.player.alive() {
            let c = self.bullet_time.toggle(&self.tuning);
            if c.started {
                self.events.push(Event::BulletTimeStarted);
            }
            if c.ended {
                self.events.push(Event::BulletTimeEnded { ran_out: false });
            }
            if c.refused {
                self.events.push(Event::BulletTimeRefused);
            }
        }
        let c = self.bullet_time.update(dt, &self.tuning);
        if c.ended {
            self.events
                .push(Event::BulletTimeEnded { ran_out: c.ran_out });
        }
        let player_scale = self.bullet_time.player_time_scale(&self.tuning);

        let aim_hit = input.camera_ray.and_then(|(o, d)| {
            let d = d.normalize_or_zero();
            self.raycast_all(world, o, d, self.tuning.aim_cast_length, false, None, false)
                .map(|(t, _)| o + d * t)
        });

        let mut shots = Vec::new();
        let ev_start = self.events.len();
        let mut ctx = StepCtx {
            tuning: &self.tuning,
            difficulty: &self.difficulty,
            weapons: &self.weapons,
            moves: &self.moves,
            bt: &mut self.bullet_time,
            rng: &mut self.rng,
            events: &mut self.events,
            shots: &mut shots,
            dt_real: dt,
            dt: dt * player_scale,
            aim_hit,
        };
        let was_alive = self.player.alive();
        self.player.step(world, input, &pressed, &mut ctx);
        self.projectiles
            .extend(shots.iter().map(Projectile::from_shot));
        if was_alive && !self.player.alive() {
            self.spawn_player_corpse();
        }
        self.listen_to_player(ev_start);

        // The world clock after the player may have changed the game speed this tick.
        let world_dt = dt * self.bullet_time.world_time_scale(&self.tuning);
        self.step_projectiles(world, world_dt);
        for t in &mut self.targets {
            t.step(world_dt);
        }
        self.step_enemies(world, world_dt);
        for c in &mut self.corpses {
            c.ragdoll.step(world, world_dt, &self.tuning);
        }

        self.prev_input = *input;
        self.time += dt as f64;
        self.world_time += world_dt as f64;
    }

    /// Take the events collected since the last call.
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Place an enemy of skin `skin` (e.g. `C01_Mafioso_A`) carrying the roster's first weapon
    /// (the Beretta). An unknown skin gets the C01 defaults.
    pub fn spawn_enemy(&mut self, pos: Vec3, yaw: f32, skin: &str) -> TargetId {
        self.spawn_enemy_with(pos, yaw, skin, 0)
    }

    /// [`Sim::spawn_enemy`] with roster weapon `weapon`.
    pub fn spawn_enemy_with(&mut self, pos: Vec3, yaw: f32, skin: &str, weapon: usize) -> TargetId {
        let skin = self.enemies.skin(skin);
        let weapon = weapon.min(self.weapons.len().saturating_sub(1));
        let mut def = self.target_def.clone();
        def.health = skin.class.health;
        def.slowmo_bonus = skin.slowmo_bonus;
        let ai = Ai::new(skin, &self.enemies, weapon, &self.weapons[weapon], yaw);
        let id = self.spawn_target_with(pos, yaw, def);
        if let Some(t) = self.targets.iter_mut().find(|t| t.id == id) {
            t.ai = Some(ai);
        }
        id
    }

    /// Make a noise enemies can hear (`group` 0..3 = MP2's loudness group one..three), pointing
    /// listeners at `interest`. The player's shots, footsteps and landings are added by the sim.
    pub fn make_noise(&mut self, pos: Vec3, group: usize, interest: Vec3) {
        self.noises.push(Noise {
            pos,
            group: group.min(2),
            interest,
            source: None,
        });
    }

    /// Place a target using [`Sim::target_def`].
    pub fn spawn_target(&mut self, pos: Vec3, yaw: f32) -> TargetId {
        let def = self.target_def.clone();
        self.spawn_target_with(pos, yaw, def)
    }

    pub fn spawn_target_with(&mut self, pos: Vec3, yaw: f32, def: TargetDef) -> TargetId {
        let id = TargetId(self.next_id);
        self.next_id += 1;
        self.targets.push(Target::new(id, pos, yaw, def));
        id
    }

    pub fn target(&self, id: TargetId) -> Option<&Target> {
        self.targets.iter().find(|t| t.id == id)
    }

    pub fn corpse(&self, owner: TargetId) -> Option<&Corpse> {
        self.corpses.iter().find(|c| c.owner == Some(owner))
    }

    /// Hurt the player (enemy fire, explosions). `dir` is the way the hit travelled.
    pub fn damage_player(&mut self, damage: f32, dir: Vec3) {
        if self
            .player
            .take_damage(damage, dir, &mut self.bullet_time, &mut self.events)
        {
            self.spawn_player_corpse();
        }
    }

    pub fn world_time_scale(&self) -> f32 {
        self.bullet_time.world_time_scale(&self.tuning)
    }

    pub fn player_time_scale(&self) -> f32 {
        self.bullet_time.player_time_scale(&self.tuning)
    }

    /// What the player's skeleton should play.
    pub fn anim(&self) -> crate::anim::AnimRequest {
        self.player
            .anim(&self.weapons, &self.tuning, self.player_time_scale())
    }

    fn spawn_player_corpse(&mut self) {
        if self.corpses.iter().any(|c| c.owner.is_none()) {
            return;
        }
        let r = Ragdoll::from_rest(
            &self.skeleton,
            self.player.pos,
            self.player.body_yaw,
            self.player.vel,
        );
        self.corpses.push(Corpse {
            owner: None,
            ragdoll: r,
        });
    }

    /// Noises the player made this tick: gunshots (hearing group one), footsteps (group two when
    /// running, three when walking) and landings (group two when hard, else three). ESTIMATE
    /// groups for everything but gunshots (`enemies.md`: "gunshots are group one").
    fn listen_to_player(&mut self, ev_start: usize) {
        let p = &self.player;
        if !p.alive() {
            return;
        }
        let chest = p.chest(&self.tuning);
        let mut out = Vec::new();
        for e in &self.events[ev_start..] {
            match e {
                Event::Shot { origin, .. } => out.push((*origin, 0)),
                Event::Landed { hard, .. } => out.push((p.pos, if *hard { 1 } else { 2 })),
                _ => {}
            }
        }
        let speed = p.vel.horizontal().length();
        if matches!(p.state, State::Ground | State::Crouch) && speed > 0.5 {
            let loud = speed >= self.enemies.tuning.loud_footstep_speed && p.state == State::Ground;
            out.push((p.pos, if loud { 1 } else { 2 }));
        }
        for (pos, group) in out {
            self.noises.push(Noise {
                pos,
                group,
                interest: chest,
                source: None,
            });
        }
    }

    fn step_enemies<W: World + ?Sized>(&mut self, world: &W, dt: f32) {
        let noises = std::mem::take(&mut self.noises);
        if !self.targets.iter().any(|t| t.ai.is_some() && t.alive()) {
            return;
        }
        let tu = &self.tuning;
        let (a, b, _) = self.player.hit_capsule(tu);
        let sense = Sense {
            alive: self.player.alive(),
            chest: self.player.chest(tu),
            points: [self.player.chest(tu), if a.y > b.y { a } else { b }],
        };
        let claimed: Vec<(TargetId, Vec3)> = self
            .targets
            .iter()
            .filter_map(|t| {
                let ai = t.ai.as_ref()?;
                Some(match ai.mode {
                    enemy::Mode::TakeCover { to, .. } => (t.id, to),
                    _ => (t.id, t.pos),
                })
            })
            .collect();
        let allies: Vec<(TargetId, Vec3)> = self
            .targets
            .iter()
            .filter(|t| t.ai.is_some() && t.alive())
            .map(|t| (t.id, t.pos))
            .collect();
        let mut covers = None;
        let mut shots = Vec::new();
        let mut alerts = Vec::new();
        let mut heard = Vec::new();
        for t in &mut self.targets {
            let mut cx = EnemyCtx {
                tuning: &self.tuning,
                data: &self.enemies,
                difficulty: &self.difficulty,
                weapons: &self.weapons,
                sense,
                noises: &noises,
                covers: &mut covers,
                claimed: &claimed,
                allies: &allies,
                rng: &mut self.rng,
                events: &mut self.events,
                shots: &mut shots,
                noises_out: &mut heard,
                alerts: &mut alerts,
                dt,
            };
            enemy::step(t, world, &mut cx);
        }
        // An enemy that spots the player tells the others nearby.
        let radius = self.enemies.tuning.alert_radius;
        for (from, at, interest) in alerts {
            for t in &mut self.targets {
                if t.id != from
                    && t.alive()
                    && t.pos.distance(at) <= radius
                    && let Some(ai) = t.ai.as_mut()
                {
                    ai.on_news(interest);
                }
            }
        }
        self.separate_enemies();
        self.projectiles.extend(shots);
        self.noises = heard;
    }

    /// Keep enemies out of each other and out of the player (horizontal push, half each way between
    /// enemies; the player is not moved).
    fn separate_enemies(&mut self) {
        let n = self.targets.len();
        let radius = |t: &Target| t.ai.as_ref().map(|a| a.skin.capsule_radius);
        for i in 0..n {
            let Some(ri) = radius(&self.targets[i]).filter(|_| self.targets[i].alive()) else {
                continue;
            };
            for j in (i + 1)..n {
                let Some(rj) = radius(&self.targets[j]).filter(|_| self.targets[j].alive()) else {
                    continue;
                };
                let d = (self.targets[i].pos - self.targets[j].pos).horizontal();
                let l = d.length();
                if l < ri + rj && l > 1e-5 {
                    let push = d * ((ri + rj - l) * 0.5 / l);
                    self.targets[i].pos += push;
                    self.targets[j].pos -= push;
                }
            }
            if self.player.alive() {
                let pr = self.tuning.capsule_radius;
                let d = (self.targets[i].pos - self.player.pos).horizontal();
                let l = d.length();
                if l < ri + pr && l > 1e-5 {
                    self.targets[i].pos += d * ((ri + pr - l) / l);
                }
            }
        }
    }

    /// Nearest hit among world, live targets (except `skip`), optionally bodies and the player's
    /// hit capsule: (distance, what).
    #[allow(clippy::too_many_arguments)]
    fn raycast_all<W: World + ?Sized>(
        &self,
        world: &W,
        o: Vec3,
        d: Vec3,
        max: f32,
        bodies: bool,
        skip: Option<TargetId>,
        player: bool,
    ) -> Option<(f32, Struck)> {
        let mut best: Option<(f32, Struck)> =
            world.raycast(o, d, max).map(|h| (h.dist, Struck::World(h)));
        if player {
            let (a, b, r) = self.player.hit_capsule(&self.tuning);
            if let Some(dist) = ray_capsule(o, d, a, b, r)
                && dist <= max
                && best.as_ref().is_none_or(|b| dist < b.0)
            {
                best = Some((dist, Struck::Player));
            }
        }
        for (i, t) in self.targets.iter().enumerate() {
            if Some(t.id) == skip {
                continue;
            }
            if let Some((dist, zone, p, n)) = t.raycast(o, d, max)
                && best.as_ref().is_none_or(|b| dist < b.0)
            {
                best = Some((dist, Struck::Target(i, zone, p, n)));
            }
        }
        if bodies {
            for (i, c) in self.corpses.iter().enumerate() {
                for p in &c.ragdoll.particles {
                    if let Some(dist) = ray_sphere(o, d, p.pos, p.radius)
                        && dist <= max
                        && best.as_ref().is_none_or(|b| dist < b.0)
                    {
                        best = Some((dist, Struck::Corpse(i, o + d * dist)));
                    }
                }
            }
        }
        best
    }

    fn step_projectiles<W: World + ?Sized>(&mut self, world: &W, dt: f32) {
        let mut projectiles = std::mem::take(&mut self.projectiles);
        projectiles.retain_mut(|p| {
            let len = p.step_length(dt);
            if len <= 0.0 {
                p.advance(0.0, dt);
                return !p.spent(self.tuning.projectile_lifetime);
            }
            let hits_player =
                p.owner.is_some() && self.player.alive() && !self.player.invulnerable();
            let hit = self.raycast_all(world, p.pos, p.dir, len, true, p.owner, hits_player);
            if p.owner.is_none() {
                // Player bullets that pass close make enemies take cover.
                let end = p.pos + p.dir * hit.as_ref().map_or(len, |h| h.0);
                let from = p.pos - p.dir * p.travelled;
                let r = self.enemies.tuning.near_miss_radius;
                for t in &mut self.targets {
                    let chest = t.to_world(Vec3::new(0.0, 1.2, 0.0));
                    if t.alive()
                        && let Some(ai) = t.ai.as_mut()
                        && enemy::segment_near(p.pos, end, chest, r)
                    {
                        ai.on_shot_at(from);
                    }
                }
            }
            match hit {
                Some((_, struck)) => {
                    self.resolve_hit(p, struck);
                    false
                }
                None => {
                    p.advance(len, dt);
                    !p.spent(self.tuning.projectile_lifetime)
                }
            }
        });
        // Keep anything spawned meanwhile (nothing spawns during this pass today).
        projectiles.append(&mut self.projectiles);
        self.projectiles = projectiles;
    }

    fn resolve_hit(&mut self, p: &Projectile, struck: Struck) {
        match struck {
            Struck::World(h) => {
                // A host-simulated target reports itself here; it applies its own damage rules.
                self.events.push(Event::Hit {
                    target: h.target,
                    zone: h.zone,
                    point: h.point,
                    normal: h.normal,
                    dir: p.dir,
                    damage: p.damage,
                });
            }
            Struck::Player => self.damage_player(p.damage, p.dir),
            Struck::Target(i, zone, point, normal) => {
                let t = &mut self.targets[i];
                let out = t.hit(p.damage, zone, point, p.dir);
                let id = t.id;
                if p.owner.is_none()
                    && let Some(ai) = t.ai.as_mut()
                {
                    ai.on_shot_at(p.pos - p.dir * p.travelled);
                }
                self.events.push(Event::Hit {
                    target: Some(id),
                    zone: Some(zone),
                    point,
                    normal,
                    dir: p.dir,
                    damage: out.damage,
                });
                self.events.push(Event::Reaction {
                    target: id,
                    reaction: out.reaction,
                });
                if out.killed {
                    let (pos, yaw, vel, bonus) = (t.pos, t.yaw, t.vel, t.def.slowmo_bonus);
                    self.events.push(Event::Kill {
                        target: id,
                        zone: Some(zone),
                    });
                    // Only the player's kills fill his meter.
                    if p.owner.is_none() {
                        let mult = self.difficulty.bullet_time_gained_per_kill_multiplier;
                        self.bullet_time.on_kill(bonus * mult, &self.tuning);
                    }
                    let mut r = Ragdoll::from_rest(&self.skeleton, pos, yaw, vel);
                    r.apply_impulse(
                        point,
                        p.dir * (p.impulse * self.tuning.ragdoll_impulse_scale),
                    );
                    self.corpses.push(Corpse {
                        owner: Some(id),
                        ragdoll: r,
                    });
                }
            }
            Struck::Corpse(i, point) => {
                let c = &mut self.corpses[i];
                c.ragdoll.apply_impulse(
                    point,
                    p.dir * (p.impulse * self.tuning.ragdoll_impulse_scale),
                );
                self.events.push(Event::BodyHit {
                    target: c.owner,
                    point,
                });
            }
        }
    }
}
