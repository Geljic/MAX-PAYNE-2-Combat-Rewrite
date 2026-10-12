//! The player: Max Payne 2's third-person controller as a fixed-tick state machine.
//!
//! How MP2 plays, and so how this works:
//! - Max always faces the aim. The stick moves him relative to it: forward runs, sideways strafes,
//!   back backpedals ([`State::Ground`]).
//! - Aiming comes from raw mouse deltas and is never slowed by bullet time. Everything else the
//!   player does (moving, animations, weapon timers) runs on the *player clock*,
//!   `dt × BulletTime::player_time_scale`, which in bullet time is slower than real time but, with
//!   kill efficiency, faster than the world's.
//! - Timed moves (jump take-off, dodge roll, shootdodge, get-up, spin reload) play a
//!   [`MoveDef`] timeline and react to its frame messages, exactly as MP2 drives them from
//!   animation data: the shootdodge slows the game at frame 10, leaves the ground with `C_Jump(5)`
//!   at frame 12 and speeds the game up again at frame 30.
//! - The shootdodge lands lying down ([`State::Prone`]); Max can keep shooting and turn all the
//!   way round, and gets up when the stick, jump or crouch is used. From
//!   `can_abort_shootdodge_frame` on, the same inputs get up straight from the dive.
//! - Crouch while moving dodges (MP2's dodge roll, `CHARANIM_DODGE*`); before
//!   `can_shootdodge_frame` the dodge can still turn into a shootdodge.
//! - Shootdodge while standing still in bullet time does the spin reload (`CHARANIM_WHIRLLEFT`).
//! - Timed moves travel along their animation's root-motion curve ([`MoveDef::distance_at`]):
//!   the dive builds speed on take-off, flies at the clip's airborne speed and slides to a stop.
//!   A dive still in the air when its clip would land holds the last airborne frame and the
//!   airborne speed until touchdown, then jumps to the clip's landing frame and slides.
//! - Shootdodge pressed in the air is the jump dive (payne option `jump_dive`): it enters the
//!   dive clip at its `C_Jump` frame, keeps the vertical speed and falls into the slide. Remedy
//!   left the HIGHSHOOTDODGE* slots empty, so this is our own move built from the ground dive.
//!
//! Health: enemy bullets and other damage go through [`Player::take_damage`]. Painkillers
//! ([`Input::painkiller`]) heal MP2's way, from `difficulty.txt`: `HealthGainedImmediately` at once,
//! then regeneration at `HealthRegenerateMultiplier` (fraction of maximum health per second) until
//! the painkiller has given `HealthGainedFromPainkiller` in all. Below the mode's `AutoHealLevel`
//! health also creeps back up to it at `AutoHealHealthIncreasePerSecond`. (Tuning's
//! `regeneration_time` is the bullet-time meter's recharge, unrelated to health.)
//!
//! Buttons in [`Input`] are *held* states sampled each tick; [`Pressed`] derives the presses.

use std::f32::consts::TAU;

use crate::anim::{AnimRequest, Clip, HitSide, MoveDir};
use crate::bullet_time::BulletTime;
use crate::enemy::Difficulty;
use crate::event::Event;
use crate::math::*;
use crate::moves::{MoveDef, MoveSet, Msg};
use crate::tuning::Tuning;
use crate::weapon::{FireMode, Hand, Shot, WeaponClass, WeaponDef, WeaponState};
use crate::world::World;

/// One tick of player input.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Stick relative to the aim: x right, y forward, each -1..1.
    pub move_axis: [f32; 2],
    /// Raw look deltas this tick (rad): yaw (+ turns left), pitch (+ looks up). Never time-scaled.
    pub look: [f32; 2],
    /// The camera's crosshair ray (origin, unit direction). The player aims where it hits the world,
    /// as MP2 does. Without it the aim follows yaw/pitch from the chest.
    pub camera_ray: Option<(Vec3, Vec3)>,
    pub fire: bool,
    pub reload: bool,
    pub jump: bool,
    pub crouch: bool,
    pub bullet_time: bool,
    pub shootdodge: bool,
    pub next_weapon: bool,
    pub prev_weapon: bool,
    /// Select an inventory slot directly (acts when it changes).
    pub select_weapon: Option<usize>,
    /// Take a painkiller (acts on the press).
    pub painkiller: bool,
}

/// Buttons that went down this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pressed {
    pub fire: bool,
    pub reload: bool,
    pub jump: bool,
    pub crouch: bool,
    pub bullet_time: bool,
    pub shootdodge: bool,
    pub next_weapon: bool,
    pub prev_weapon: bool,
    pub select_weapon: Option<usize>,
    pub painkiller: bool,
}

impl Pressed {
    pub fn between(prev: &Input, cur: &Input) -> Pressed {
        Pressed {
            fire: cur.fire && !prev.fire,
            reload: cur.reload && !prev.reload,
            jump: cur.jump && !prev.jump,
            crouch: cur.crouch && !prev.crouch,
            bullet_time: cur.bullet_time && !prev.bullet_time,
            shootdodge: cur.shootdodge && !prev.shootdodge,
            next_weapon: cur.next_weapon && !prev.next_weapon,
            prev_weapon: cur.prev_weapon && !prev.prev_weapon,
            painkiller: cur.painkiller && !prev.painkiller,
            select_weapon: if cur.select_weapon != prev.select_weapon {
                cur.select_weapon
            } else {
                None
            },
        }
    }
}

/// The player's state. Times are player-clock seconds since the state began.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// Standing, running, strafing or backpedalling.
    Ground,
    Crouch,
    /// Jump wind-up until the animation's `C_Jump` message. `forward`: running jump.
    JumpStart {
        t: f32,
        forward: bool,
    },
    /// Airborne. `jumped` false when walked off an edge.
    Air {
        jumped: bool,
    },
    Land {
        t: f32,
        dur: f32,
        hard: bool,
    },
    /// MP2's dodge roll in a direction relative to the aim.
    Dodge {
        t: f32,
        dir: MoveDir,
        world_dir: Vec3,
        airborne: bool,
    },
    /// The dive. `slowmo`: the meter allowed slow motion; `airborne` after `C_Jump`; `landed` after
    /// touching down again.
    Shootdodge {
        t: f32,
        dir: MoveDir,
        world_dir: Vec3,
        slowmo: bool,
        airborne: bool,
        landed: bool,
    },
    /// Lying after a dive (in `dir`).
    Prone {
        t: f32,
        dir: MoveDir,
    },
    GetUp {
        t: f32,
        dir: MoveDir,
    },
    /// Bullet-time spin reload, starting from `yaw0`.
    Spin {
        t: f32,
        yaw0: f32,
    },
    Dead {
        t: f32,
    },
}

impl State {
    pub fn name(&self) -> &'static str {
        match self {
            State::Ground => "ground",
            State::Crouch => "crouch",
            State::JumpStart { .. } => "jump start",
            State::Air { .. } => "air",
            State::Land { .. } => "land",
            State::Dodge { .. } => "dodge",
            State::Shootdodge { .. } => "shootdodge",
            State::Prone { .. } => "prone",
            State::GetUp { .. } => "get up",
            State::Spin { .. } => "spin",
            State::Dead { .. } => "dead",
        }
    }

    /// May the player shoot in this state? Not while rolling, getting up, flinching or dead.
    pub fn can_fire(&self) -> bool {
        !matches!(
            self,
            State::Dodge { .. } | State::GetUp { .. } | State::Dead { .. }
        )
    }
}

/// Everything a player step reads or writes besides the player itself.
pub struct StepCtx<'a> {
    pub tuning: &'a Tuning,
    pub difficulty: &'a Difficulty,
    pub weapons: &'a [WeaponDef],
    pub moves: &'a MoveSet,
    pub bt: &'a mut BulletTime,
    pub rng: &'a mut Rng,
    pub events: &'a mut Vec<Event>,
    pub shots: &'a mut Vec<Shot>,
    /// Real seconds this tick (aim and body turning).
    pub dt_real: f32,
    /// Player-clock seconds this tick.
    pub dt: f32,
    /// Where the camera ray hit (world or a target), if it did.
    pub aim_hit: Option<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// Feet position.
    pub pos: Vec3,
    pub vel: Vec3,
    /// Aim yaw / pitch (rad). Max faces `yaw` except during dives, prone and spins.
    pub yaw: f32,
    pub pitch: f32,
    /// Recoil kick added to the aim pitch; recovers over time.
    pub recoil: f32,
    pub body_yaw: f32,
    pub state: State,
    pub health: f32,
    /// Painkillers carried.
    pub painkillers: u32,
    /// Health still to come from painkillers already taken.
    pub heal_pending: f32,
    /// Carried weapons and the selected one.
    pub inventory: Vec<WeaponState>,
    pub current: usize,
    /// Seconds left of a weapon switch (no firing meanwhile).
    pub switch_timer: f32,
    /// Where the weapon points: the crosshair hit, kept at least `aim_min_distance` away.
    pub aim_target: Vec3,
    /// Crouch toggle (also restored after flinching).
    pub crouched: bool,
    /// Animation-driven protection (`C_SetDodgeImmortal`, `C_EnableProjectileCollisions(false)`).
    pub immortal: bool,
    pub projectile_collisions: bool,
    /// The last hit: player seconds since it landed and the side it came from. MP2 does not stun
    /// Max when he is shot (control and the current move carry on), so this only drives a visual
    /// flinch the host may layer on top.
    pub last_hit: Option<(f32, HitSide)>,
    /// Highest point of the current airborne phase.
    pub fall_from: f32,
    /// No jump dive until the feet touch ground again: set by a jump dive (one per airtime) and
    /// when a dodge roll's hop carries on into a fall (the roll has its own window,
    /// `can_shootdodge_frame`, to become a dive).
    air_dive_blocked: bool,
    since_shot: f32,
    anim_clip: Clip,
    anim_time: f32,
}

/// What the stick asks for this tick.
#[derive(Clone, Copy, Debug)]
struct Wish {
    /// Local (right, forward), length ≤ 1, zero inside the deadzone.
    local: [f32; 2],
    /// World direction scaled by deflection.
    world: Vec3,
}

impl Wish {
    fn active(&self) -> bool {
        self.world.length_sq() > 0.0
    }
    fn dir(&self) -> MoveDir {
        MoveDir::from_local(self.local[0], self.local[1])
    }
}

impl Player {
    /// A player standing at `pos` facing `yaw`, carrying every weapon in `weapons` with full
    /// magazines and half of each pocket in reserve.
    pub fn new(pos: Vec3, yaw: f32, weapons: &[WeaponDef], tu: &Tuning) -> Player {
        let inventory = weapons
            .iter()
            .enumerate()
            .map(|(i, d)| WeaponState::new(i, d, d.pocket / 2))
            .collect();
        Player {
            pos,
            vel: Vec3::ZERO,
            yaw,
            pitch: 0.0,
            recoil: 0.0,
            body_yaw: yaw,
            state: State::Ground,
            health: tu.max_health,
            painkillers: tu.painkillers_start.max(0.0) as u32,
            heal_pending: 0.0,
            inventory,
            current: 0,
            switch_timer: 0.0,
            aim_target: pos + Vec3::Y * tu.muzzle_stand + yaw_dir(yaw) * tu.aim_cast_length,
            crouched: false,
            immortal: false,
            projectile_collisions: true,
            last_hit: None,
            fall_from: pos.y,
            air_dive_blocked: false,
            since_shot: f32::MAX,
            anim_clip: Clip::Idle,
            anim_time: 0.0,
        }
    }

    pub fn alive(&self) -> bool {
        !matches!(self.state, State::Dead { .. })
    }

    /// Takes no damage right now (animation-driven dodge immunity).
    pub fn invulnerable(&self) -> bool {
        self.immortal || !self.projectile_collisions
    }

    pub fn weapon(&self) -> &WeaponState {
        &self.inventory[self.current]
    }

    /// Chest point the aim is measured from.
    pub fn chest(&self, tu: &Tuning) -> Vec3 {
        self.pos + Vec3::Y * self.muzzle_height(tu)
    }

    /// Aim direction from yaw/pitch plus recoil.
    pub fn aim_dir(&self, tu: &Tuning) -> Vec3 {
        aim_dir(
            self.yaw,
            (self.pitch + self.recoil).clamp(tu.pitch_min, tu.pitch_max),
        )
    }

    fn muzzle_height(&self, tu: &Tuning) -> f32 {
        match self.state {
            State::Crouch | State::Dodge { .. } => tu.muzzle_crouch,
            State::Prone { .. } | State::GetUp { .. } => tu.muzzle_prone,
            State::Shootdodge {
                airborne: true,
                landed: false,
                ..
            } => tu.muzzle_crouch * 0.6,
            State::Shootdodge { landed: true, .. } => tu.muzzle_prone,
            _ => tu.muzzle_stand,
        }
    }

    /// Where a bullet from `hand` leaves.
    pub fn muzzle(&self, hand: Hand, tu: &Tuning) -> Vec3 {
        let side = if hand == Hand::Right { 1.0 } else { -1.0 };
        self.chest(tu) + yaw_right(self.yaw) * (tu.muzzle_side * side)
    }

    fn capsule_top(&self, tu: &Tuning) -> f32 {
        match self.state {
            State::Crouch | State::Dodge { .. } => tu.crouch_top,
            State::Prone { .. }
            | State::GetUp { .. }
            | State::Shootdodge { .. }
            | State::Dead { .. } => tu.prone_top,
            _ => tu.capsule_top,
        }
    }

    fn wish(&self, input: &Input, tu: &Tuning) -> Wish {
        let [mut x, mut y] = input.move_axis;
        let l = (x * x + y * y).sqrt();
        if l < tu.deadzone {
            return Wish {
                local: [0.0; 2],
                world: Vec3::ZERO,
            };
        }
        if l > 1.0 {
            x /= l;
            y /= l;
        }
        Wish {
            local: [x, y],
            world: yaw_right(self.yaw) * x + yaw_dir(self.yaw) * y,
        }
    }

    // --- the step ---------------------------------------------------------------------------

    /// Advance one tick.
    pub fn step<W: World + ?Sized>(
        &mut self,
        world: &W,
        input: &Input,
        pressed: &Pressed,
        ctx: &mut StepCtx,
    ) {
        let tu = ctx.tuning;
        if self.alive() {
            // Raw mouse: real time, never scaled.
            self.yaw = wrap_angle(self.yaw + input.look[0]);
            self.pitch = (self.pitch + input.look[1]).clamp(tu.pitch_min, tu.pitch_max);
            self.switch_weapons(pressed, ctx);
            if pressed.painkiller {
                self.use_painkiller(ctx);
            }
            self.regenerate(ctx);
        }
        self.step_state(world, input, pressed, ctx);
        self.turn_body(ctx);
        self.update_aim_target(input, ctx.aim_hit, ctx.tuning);
        self.step_weapon(input, pressed, ctx);
        self.track_anim(ctx.dt);
        if let Some((t, _)) = self.last_hit.as_mut() {
            *t += ctx.dt;
        }
    }

    fn step_state<W: World + ?Sized>(
        &mut self,
        world: &W,
        input: &Input,
        pressed: &Pressed,
        ctx: &mut StepCtx,
    ) {
        let tu = ctx.tuning;
        let moves = ctx.moves;
        let dt = ctx.dt;
        let wish = self.wish(input, tu);
        // The jump dive: shootdodge pressed while airborne (jumping or falling).
        if matches!(self.state, State::Air { .. })
            && pressed.shootdodge
            && tu.jump_dive > 0.5
            && !self.air_dive_blocked
            && self.air_shootdodge(wish, ctx)
        {
            return;
        }
        match self.state {
            State::Ground => {
                if pressed.shootdodge {
                    self.ground_shootdodge(wish, ctx);
                } else if pressed.jump {
                    self.state = State::JumpStart {
                        t: 0.0,
                        forward: wish.active(),
                    };
                } else if pressed.crouch {
                    if wish.active() && self.vel.horizontal().length() >= tu.roll_min_speed {
                        self.start_dodge(wish, ctx);
                    } else {
                        self.crouched = true;
                        self.state = State::Crouch;
                    }
                } else {
                    self.locomote(
                        world,
                        wish,
                        (tu.run_speed, tu.strafe_speed, tu.backpedal_speed),
                        tu,
                        dt,
                    );
                }
            }
            State::Crouch => {
                if pressed.shootdodge {
                    self.ground_shootdodge(wish, ctx);
                } else if pressed.crouch || pressed.jump {
                    self.crouched = false;
                    self.state = State::Ground;
                } else {
                    let s = tu.crouch_speed;
                    self.locomote(world, wish, (s, s, s), tu, dt);
                }
            }
            State::JumpStart { .. } if pressed.shootdodge => {
                // Still on the ground during the wind-up: an ordinary dive.
                self.ground_shootdodge(wish, ctx);
            }
            State::JumpStart { t, forward } => {
                let def = if forward {
                    &moves.jump_forward
                } else {
                    &moves.jump_up
                };
                let t1 = t + dt;
                let jump = self.run_msgs(def, t, t1, true, ctx);
                let has_jump = def.first_time(|m| matches!(m, Msg::Jump(_))).is_some();
                let take_off =
                    jump.or((!has_jump).then_some(tu.jump_speed / tu.jump_scale.max(1e-4)));
                if let Some(v) = take_off {
                    self.vel.y = v * tu.jump_scale;
                    self.fall_from = self.pos.y;
                    self.state = State::Air { jumped: true };
                    ctx.events.push(Event::Jumped);
                } else {
                    let (f, s, b) = if forward {
                        (tu.run_speed, tu.strafe_speed, tu.backpedal_speed)
                    } else {
                        (0.0, 0.0, 0.0)
                    };
                    self.locomote(world, wish, (f, s, b), tu, dt);
                    if let State::JumpStart { .. } = self.state {
                        self.state = if t1 >= def.duration() {
                            State::Air { jumped: false }
                        } else {
                            State::JumpStart { t: t1, forward }
                        };
                    }
                }
            }
            State::Air { .. } => {
                let vh = self.vel.horizontal();
                let target = if wish.active() {
                    wish.world.normalize_or_zero() * vh.length().max(tu.airborne_speed)
                } else {
                    vh
                };
                let vh = approach_vec(vh, target, tu.air_accel * dt);
                self.vel = Vec3::new(vh.x, self.vel.y, vh.z);
                if let Some(drop) = self.air_step(world, tu, dt) {
                    let hard = drop >= tu.hard_land_height;
                    if hard {
                        self.vel = Vec3::ZERO;
                    }
                    let dur = if hard {
                        tu.hard_land_time
                    } else {
                        tu.land_time
                    };
                    self.state = State::Land { t: 0.0, dur, hard };
                    ctx.events.push(Event::Landed { drop, hard });
                }
            }
            State::Land { t, dur, hard } => {
                if hard {
                    self.locomote(world, wish, (0.0, 0.0, 0.0), tu, dt);
                } else {
                    self.locomote(
                        world,
                        wish,
                        (tu.run_speed, tu.strafe_speed, tu.backpedal_speed),
                        tu,
                        dt,
                    );
                }
                if let State::Land { .. } = self.state {
                    let t1 = t + dt;
                    self.state = if t1 >= dur {
                        self.standing_state()
                    } else {
                        State::Land { t: t1, dur, hard }
                    };
                }
            }
            State::Dodge {
                t,
                dir,
                world_dir,
                mut airborne,
            } => {
                let def = &moves.dodge[dir.index()];
                let t1 = t + dt;
                if pressed.shootdodge && def.frame_at(t) < tu.can_shootdodge_frame {
                    let w = Wish {
                        local: self.wish(input, tu).local,
                        world: world_dir,
                    };
                    self.start_shootdodge(w, dir, ctx);
                    return;
                }
                let jump = self.run_msgs(def, t, t1, true, ctx);
                let vh = world_dir * def.speed_between(t, t1);
                self.vel = Vec3::new(vh.x, self.vel.y, vh.z);
                if let Some(v) = jump {
                    self.vel.y = v * tu.jump_scale;
                    self.fall_from = self.pos.y;
                    airborne = true;
                }
                if airborne {
                    if self.air_step(world, tu, dt).is_some() {
                        airborne = false;
                    }
                } else {
                    self.move_horizontal(world, vh * dt, tu, tu.step_up);
                    if !self.follow_ground(world, tu) {
                        airborne = true;
                        self.fall_from = self.pos.y;
                    }
                }
                self.state = if t1 >= def.duration() {
                    if airborne {
                        self.air_dive_blocked = true;
                        State::Air { jumped: true }
                    } else {
                        self.vel *= 0.5;
                        State::Ground
                    }
                } else {
                    State::Dodge {
                        t: t1,
                        dir,
                        world_dir,
                        airborne,
                    }
                };
            }
            State::Shootdodge {
                t,
                dir,
                world_dir,
                slowmo,
                mut airborne,
                mut landed,
            } => {
                let def = &moves.shootdodge[dir.index()];
                let window = def.air_window(tu.gravity, tu.jump_scale);
                let flying = airborne && !landed;
                // In flight the clip plays up to the frame before its landing frame and holds
                // there until touchdown, flying on at the clip's airborne speed. Falling during
                // the slide (off a ledge) holds the current frame and keeps the momentum.
                let (hold_t, hold_speed) = match window {
                    Some((t_jump, t_land)) if flying && t < t_land => (
                        (t_land - 1.0 / def.fps.max(1e-6)).max(t_jump),
                        def.speed_between(t_jump, t_land),
                    ),
                    Some(_) if flying => (t, self.vel.horizontal().length()),
                    None if flying => (def.duration(), self.vel.horizontal().length()),
                    _ => (f32::INFINITY, 0.0),
                };
                let mut t1 = (t + dt).min(hold_t.max(t));
                let jump = self.run_msgs(def, t, t1, slowmo, ctx);
                // Root motion: the curve's distance over the clip time that passed, plus the
                // held speed for any time the clip stood still.
                let speed = (def.distance_at(t1) - def.distance_at(t)
                    + hold_speed * (dt - (t1 - t)).max(0.0))
                    / dt.max(1e-6);
                let vh = world_dir * speed;
                self.vel = Vec3::new(vh.x, self.vel.y, vh.z);
                if let Some(v) = jump {
                    self.vel.y = v * tu.jump_scale;
                    self.fall_from = self.pos.y;
                    airborne = true;
                    landed = false;
                }
                if airborne && !landed {
                    if let Some(drop) = self.air_step(world, tu, dt) {
                        landed = true;
                        ctx.events.push(Event::ShootdodgeLanded);
                        ctx.events.push(Event::Landed { drop, hard: false });
                        // Touchdown: carry on from the clip's landing frame (slide, lie down).
                        if let Some((_, t_land)) = window
                            && t1 < t_land
                        {
                            self.run_msgs(def, t1, t_land, slowmo, ctx);
                            t1 = t_land;
                        }
                    }
                } else {
                    self.move_horizontal(world, self.vel.horizontal() * dt, tu, 0.1);
                    if !self.follow_ground(world, tu) {
                        // Off an edge: fall the rest of the way as part of the dive.
                        airborne = true;
                        landed = false;
                        self.fall_from = self.pos.y;
                    }
                }
                let grounded = landed || !airborne;
                let abort = grounded
                    && def.frame_at(t1) >= tu.can_abort_shootdodge_frame
                    && (wish.active() || pressed.jump || pressed.crouch);
                if abort {
                    self.start_getup(dir, ctx);
                } else if t1 >= def.duration() && grounded {
                    // The curve has already slid the dive to a stop; Prone keeps what is left
                    // (≈0 with game data, the constant speed without it).
                    self.vel = world_dir * def.end_speed();
                    self.state = State::Prone { t: 0.0, dir };
                } else {
                    self.state = State::Shootdodge {
                        t: t1,
                        dir,
                        world_dir,
                        slowmo,
                        airborne,
                        landed,
                    };
                }
            }
            State::Prone { t, dir } => {
                let vh = approach_vec(self.vel.horizontal(), Vec3::ZERO, tu.prone_slide_decel * dt);
                self.vel = vh;
                self.move_horizontal(world, vh * dt, tu, 0.1);
                if !self.follow_ground(world, tu) {
                    self.enter_fall();
                } else if wish.active() || pressed.jump || pressed.crouch {
                    self.start_getup(dir, ctx);
                } else {
                    self.state = State::Prone { t: t + dt, dir };
                }
            }
            State::GetUp { t, dir } => {
                let def = &moves.getup[dir.index()];
                let t1 = t + dt;
                self.run_msgs(def, t, t1, true, ctx);
                self.vel = Vec3::ZERO;
                self.root_motion(world, def, t, t1, self.body_yaw, tu, dt);
                if !self.follow_ground(world, tu) {
                    self.enter_fall();
                } else if t1 >= def.duration() {
                    self.state = State::Ground;
                    self.crouched = false;
                    ctx.events.push(Event::GotUp);
                } else {
                    self.state = State::GetUp { t: t1, dir };
                }
            }
            State::Spin { t, yaw0 } => {
                let def = &moves.spin;
                let t1 = t + dt;
                self.run_msgs(def, t, t1, true, ctx);
                if def.has_motion() {
                    self.vel = Vec3::ZERO;
                    self.root_motion(world, def, t, t1, yaw0, tu, dt);
                    if !self.follow_ground(world, tu) {
                        self.enter_fall();
                    }
                } else {
                    self.locomote(
                        world,
                        Wish {
                            local: [0.0; 2],
                            world: Vec3::ZERO,
                        },
                        (0.0, 0.0, 0.0),
                        tu,
                        dt,
                    );
                }
                if let State::Spin { .. } = self.state {
                    self.state = if t1 >= def.duration() {
                        State::Ground
                    } else {
                        State::Spin { t: t1, yaw0 }
                    };
                }
            }
            State::Dead { t } => {
                self.vel = approach_vec(self.vel.horizontal(), Vec3::ZERO, tu.decel * dt);
                self.follow_ground(world, tu);
                self.state = State::Dead { t: t + dt };
            }
        }
    }

    fn standing_state(&self) -> State {
        if self.crouched {
            State::Crouch
        } else {
            State::Ground
        }
    }

    fn enter_fall(&mut self) {
        self.vel.y = self.vel.y.min(0.0);
        self.fall_from = self.pos.y;
        self.state = State::Air { jumped: false };
    }

    /// Shootdodge pressed on foot: dive the way the stick points; standing still, spin-reload in
    /// bullet time, or dive forward outside it.
    fn ground_shootdodge(&mut self, wish: Wish, ctx: &mut StepCtx) {
        let def = &ctx.weapons[self.inventory[self.current].def];
        if !def.shootdodge_allowed {
            return;
        }
        if wish.active() {
            self.start_shootdodge(wish, wish.dir(), ctx);
        } else if ctx.bt.active {
            self.state = State::Spin {
                t: 0.0,
                yaw0: self.body_yaw,
            };
            ctx.events.push(Event::Spin);
        } else {
            let w = Wish {
                local: [0.0, 1.0],
                world: yaw_dir(self.yaw),
            };
            self.start_shootdodge(w, MoveDir::Forward, ctx);
        }
    }

    fn start_shootdodge(&mut self, wish: Wish, dir: MoveDir, ctx: &mut StepCtx) {
        // The meter check and penalty still apply; the option only decides whether the dive's own
        // game-speed messages are obeyed.
        let slowmo = ctx.bt.begin_shootdodge(ctx.tuning) && ctx.tuning.shootdodge_slowmo > 0.5;
        let world_dir = wish.world.normalize_or_zero();
        self.body_yaw = self.yaw;
        self.crouched = false;
        self.state = State::Shootdodge {
            t: 0.0,
            dir,
            world_dir,
            slowmo,
            airborne: false,
            landed: false,
        };
        ctx.events.push(Event::ShootdodgeStarted { slowmo });
    }

    /// Shootdodge pressed in the air: the jump dive. Enters the dive clip at its `C_Jump` frame,
    /// already airborne, keeping the vertical speed (plus `jump_dive_boost`) and flying at the
    /// clip's airborne speed. The messages before the entry frame run at once except the C_Jump:
    /// the frame-0 `C_Reload` (MP2's free reload on a dive) and, when the dive slows the world,
    /// its frame-10 game-speed change. Returns false when no dive starts.
    fn air_shootdodge(&mut self, wish: Wish, ctx: &mut StepCtx) -> bool {
        let tu = ctx.tuning;
        let moves = ctx.moves;
        if !ctx.weapons[self.inventory[self.current].def].shootdodge_allowed {
            return false;
        }
        let (wish, dir) = if wish.active() {
            (wish, wish.dir())
        } else {
            let fwd = Wish {
                local: [0.0, 1.0],
                world: yaw_dir(self.yaw),
            };
            (fwd, MoveDir::Forward)
        };
        let def = &moves.shootdodge[dir.index()];
        let Some((t_jump, t_land)) = def.air_window(tu.gravity, tu.jump_scale) else {
            return false;
        };
        let slowmo = ctx.bt.begin_shootdodge(tu) && tu.shootdodge_slowmo > 0.5;
        // Just past the take-off frame, so its messages (bullet time off and blocked) run now
        // and the C_Jump is dropped, not fired again on the next step.
        let t0 = t_jump + 1e-4;
        let _skipped_jump = self.run_msgs(def, 0.0, t0, slowmo, ctx);
        let world_dir = wish.world.normalize_or_zero();
        let vh = world_dir * def.speed_between(t_jump, t_land);
        self.vel = Vec3::new(vh.x, self.vel.y + tu.jump_dive_boost, vh.z);
        self.body_yaw = self.yaw;
        self.crouched = false;
        self.air_dive_blocked = true;
        self.state = State::Shootdodge {
            t: t0,
            dir,
            world_dir,
            slowmo,
            airborne: true,
            landed: false,
        };
        ctx.events.push(Event::ShootdodgeStarted { slowmo });
        true
    }

    /// Move along a move's root-motion curve from `t0` to `t1`, in its measured clip direction
    /// turned by `yaw` (forward without one). For moves with no stick direction (get-up, spin).
    #[allow(clippy::too_many_arguments)]
    fn root_motion<W: World + ?Sized>(
        &mut self,
        world: &W,
        def: &MoveDef,
        t0: f32,
        t1: f32,
        yaw: f32,
        tu: &Tuning,
        dt: f32,
    ) {
        if !def.has_motion() {
            return;
        }
        let [x, z] = def.root_dir.unwrap_or([0.0, 1.0]);
        let dir = yaw_right(yaw) * x + yaw_dir(yaw) * z;
        let d = def.distance_at(t1) - def.distance_at(t0);
        self.vel = dir * (d / dt.max(1e-6));
        self.move_horizontal(world, dir * d, tu, tu.step_up);
    }

    fn start_dodge(&mut self, wish: Wish, ctx: &mut StepCtx) {
        self.state = State::Dodge {
            t: 0.0,
            dir: wish.dir(),
            world_dir: wish.world.normalize_or_zero(),
            airborne: false,
        };
        ctx.events.push(Event::Dodged);
    }

    fn start_getup(&mut self, dir: MoveDir, ctx: &mut StepCtx) {
        self.vel = Vec3::ZERO;
        self.state = State::GetUp { t: 0.0, dir };
        ctx.events.push(Event::GettingUp);
    }

    /// Act on a move's messages in [t0, t1). `game_speed` false ignores `Gm_ChangeGameSpeed`
    /// (a shootdodge without meter runs in real time). Returns a `C_Jump` speed if one fired.
    fn run_msgs(
        &mut self,
        def: &MoveDef,
        t0: f32,
        t1: f32,
        game_speed: bool,
        ctx: &mut StepCtx,
    ) -> Option<f32> {
        let mut jump = None;
        for m in def.msgs_between(t0, t1) {
            match m {
                Msg::GameSpeed { speed, blend } => {
                    if game_speed {
                        ctx.bt.change_game_speed(*speed, *blend);
                    }
                }
                Msg::Jump(v) => jump = Some(*v),
                Msg::Reload => self.instant_reload(ctx),
                // Without shootdodge slow motion the dive leaves bullet time as it is too:
                // MP2's dive switches bullet time off at take-off because its own game speed
                // takes over, which would only speed the world up mid-air here.
                Msg::EnableBulletTime(_) | Msg::BulletTimeBlock(_) if !game_speed => {}
                Msg::EnableBulletTime(on) => {
                    let c = ctx.bt.set_active(*on);
                    if c.ended {
                        ctx.events.push(Event::BulletTimeEnded { ran_out: false });
                    }
                    if c.started {
                        ctx.events.push(Event::BulletTimeStarted);
                    }
                }
                Msg::BulletTimeBlock(on) => ctx.bt.blocked = *on,
                Msg::ProjectileCollisions(on) => self.projectile_collisions = *on,
                Msg::Immortal(on) => self.immortal = *on,
                Msg::Other(s) => ctx.events.push(Event::AnimMessage(s.clone())),
            }
        }
        jump
    }

    /// `C_Reload()`: one reload cycle's rounds, instantly.
    fn instant_reload(&mut self, ctx: &mut StepCtx) {
        let ws = &mut self.inventory[self.current];
        let def = &ctx.weapons[ws.def];
        ws.cancel_reload();
        let before = ws.mag;
        ws.load_rounds(def);
        if ws.mag != before {
            ctx.events.push(Event::ReloadDone { weapon: ws.def });
        }
    }

    // --- movement helpers -------------------------------------------------------------------

    /// Ground locomotion: accelerate towards the stick's speed (forward / strafe / back speeds),
    /// move with wall sliding, follow the ground or start falling.
    fn locomote<W: World + ?Sized>(
        &mut self,
        world: &W,
        wish: Wish,
        speeds: (f32, f32, f32),
        tu: &Tuning,
        dt: f32,
    ) {
        let (fwd, strafe, back) = speeds;
        let [x, y] = wish.local;
        let target = yaw_right(self.yaw) * (x * strafe)
            + yaw_dir(self.yaw) * (y * if y >= 0.0 { fwd } else { back });
        let cur = self.vel.horizontal();
        let rate =
            if target.length_sq() < 1e-8 || cur.dot(target) < 0.0 || target.length() < cur.length()
            {
                tu.decel
            } else {
                tu.accel
            };
        let vh = approach_vec(cur, target, rate * dt);
        self.vel = vh;
        self.move_horizontal(world, vh * dt, tu, tu.step_up);
        if !self.follow_ground(world, tu) {
            self.enter_fall();
        }
    }

    /// Move horizontally in short sub-steps, pushing out of walls and removing velocity into them.
    fn move_horizontal<W: World + ?Sized>(
        &mut self,
        world: &W,
        delta: Vec3,
        tu: &Tuning,
        step: f32,
    ) {
        let len = delta.length();
        if len < 1e-9 {
            return;
        }
        let r = tu.capsule_radius;
        let n = (len / (r * 0.5)).ceil().max(1.0) as usize;
        let top = self.capsule_top(tu);
        let mut total = Vec3::ZERO;
        for _ in 0..n {
            self.pos += delta / n as f32;
            let lo = self.pos.y + step + r;
            let hi = (self.pos.y + top - r).max(lo);
            let mut push = world.push_out(
                Vec3::new(self.pos.x, lo, self.pos.z),
                Vec3::new(self.pos.x, hi, self.pos.z),
                r,
            );
            push.y = 0.0;
            self.pos += push;
            total += push;
        }
        let nrm = total.normalize_or_zero();
        let vn = self.vel.dot(nrm);
        if vn < 0.0 {
            self.vel -= nrm * vn;
        }
    }

    fn follow_ground<W: World + ?Sized>(&mut self, world: &W, tu: &Tuning) -> bool {
        match world.ground(
            self.pos.x,
            self.pos.z,
            self.pos.y - tu.snap_down,
            self.pos.y + tu.step_up,
        ) {
            Some(g) => {
                self.pos.y = g.height;
                self.vel.y = 0.0;
                self.air_dive_blocked = false;
                true
            }
            None => false,
        }
    }

    /// Ballistic step. Returns the drop height when it lands.
    fn air_step<W: World + ?Sized>(&mut self, world: &W, tu: &Tuning, dt: f32) -> Option<f32> {
        // MP2's character controller moves by the average of the old and new vertical speed
        // (X_PhysicalSimulationMFC), which follows the ballistic arc exactly.
        let vy0 = self.vel.y;
        self.vel.y -= tu.gravity * dt;
        self.move_horizontal(world, self.vel.horizontal() * dt, tu, 0.05);
        let old_y = self.pos.y;
        self.pos.y += 0.5 * (vy0 + self.vel.y) * dt;
        self.fall_from = self.fall_from.max(self.pos.y);
        if self.vel.y <= 0.0
            && let Some(g) = world.ground(self.pos.x, self.pos.z, self.pos.y, old_y + 0.01)
        {
            self.pos.y = g.height;
            self.vel.y = 0.0;
            self.air_dive_blocked = false;
            return Some(self.fall_from - self.pos.y);
        }
        None
    }

    fn turn_body(&mut self, ctx: &StepCtx) {
        let tu = ctx.tuning;
        match self.state {
            State::Shootdodge { .. }
            | State::Dodge { .. }
            | State::GetUp { .. }
            | State::Dead { .. } => {}
            State::Prone { .. } => {
                // The upper body twists up to prone_twist; beyond that the body rolls round to follow.
                let rel = wrap_angle(self.yaw - self.body_yaw);
                let excess = rel - rel.clamp(-tu.prone_twist, tu.prone_twist);
                if excess != 0.0 {
                    let step = excess.clamp(
                        -tu.prone_turn_rate * ctx.dt_real,
                        tu.prone_turn_rate * ctx.dt_real,
                    );
                    self.body_yaw = wrap_angle(self.body_yaw + step);
                }
            }
            State::Spin { t, yaw0 } => {
                let u = (t / ctx.moves.spin.duration().max(1e-4)).min(1.0);
                self.body_yaw = wrap_angle(yaw0 + TAU * u);
            }
            _ => {
                self.body_yaw =
                    approach_angle(self.body_yaw, self.yaw, tu.body_turn_rate * ctx.dt_real)
            }
        }
    }

    /// MP2's update2DAiming: aim at the crosshair hit, pushed out along the camera ray to at least
    /// `aim_min_distance` from the character.
    fn update_aim_target(&mut self, input: &Input, hit: Option<Vec3>, tu: &Tuning) {
        let chest = self.chest(tu);
        let Some((o, d)) = input.camera_ray else {
            self.aim_target = chest + self.aim_dir(tu) * tu.aim_cast_length;
            return;
        };
        let d = d.normalize_or_zero();
        let p = hit.unwrap_or(o + d * tu.aim_cast_length);
        if p.distance(chest) >= tu.aim_min_distance {
            self.aim_target = p;
            return;
        }
        // Solve |o + d t − chest| = min for the far root.
        let oc = o - chest;
        let b = oc.dot(d);
        let c = oc.length_sq() - tu.aim_min_distance * tu.aim_min_distance;
        let h = (b * b - c).max(0.0);
        self.aim_target = o + d * (-b + h.sqrt());
    }

    // --- weapons ----------------------------------------------------------------------------

    fn switch_weapons(&mut self, pressed: &Pressed, ctx: &mut StepCtx) {
        let n = self.inventory.len();
        if n == 0 {
            return;
        }
        let to = if let Some(i) = pressed.select_weapon.filter(|&i| i < n) {
            i
        } else if pressed.next_weapon {
            (self.current + 1) % n
        } else if pressed.prev_weapon {
            (self.current + n - 1) % n
        } else {
            return;
        };
        if to == self.current {
            return;
        }
        let old = &mut self.inventory[self.current];
        if old.cancel_reload() {
            ctx.events.push(Event::ReloadCancelled { weapon: old.def });
        }
        let from = old.def;
        self.current = to;
        self.switch_timer = ctx.tuning.switch_time;
        ctx.events.push(Event::WeaponSwitched {
            from,
            to: self.inventory[to].def,
        });
    }

    fn stance_spread(&self, tu: &Tuning) -> f32 {
        match self.state {
            State::Ground if self.vel.horizontal().length() > 0.5 => tu.spread_moving,
            State::Crouch => tu.spread_crouch,
            State::Air { .. } | State::JumpStart { .. } => tu.spread_air,
            State::Prone { .. } => tu.spread_prone,
            _ => 1.0,
        }
    }

    fn step_weapon(&mut self, input: &Input, pressed: &Pressed, ctx: &mut StepCtx) {
        let tu = ctx.tuning;
        let dt = ctx.dt;
        self.switch_timer = (self.switch_timer - dt).max(0.0);
        self.recoil = (self.recoil - tu.recoil_recovery * dt).max(0.0);
        self.since_shot = (self.since_shot + dt).min(1e6);
        if self.inventory.is_empty() {
            return;
        }
        let cur = self.current;
        let def = &ctx.weapons[self.inventory[cur].def];
        if self.inventory[cur].tick(def, dt) {
            ctx.events.push(Event::ReloadDone {
                weapon: self.inventory[cur].def,
            });
        }
        if !self.alive() {
            return;
        }
        if pressed.reload && self.switch_timer <= 0.0 && self.inventory[cur].start_reload(def) {
            ctx.events.push(Event::ReloadStarted {
                weapon: self.inventory[cur].def,
            });
        }

        let can = self.state.can_fire() && self.switch_timer <= 0.0;
        if !can || !input.fire {
            // No banking of fire time while not shooting.
            let ws = &mut self.inventory[cur];
            ws.cooldown = ws.cooldown.max(0.0);
        }
        let trigger = match def.fire_mode {
            FireMode::Semi => pressed.fire,
            FireMode::Auto => input.fire,
        };
        if !(can && trigger) || self.inventory[cur].reload.is_some() {
            return;
        }
        if self.inventory[cur].mag == 0 {
            if self.inventory[cur].start_reload(def) {
                ctx.events.push(Event::ReloadStarted {
                    weapon: self.inventory[cur].def,
                });
            } else if pressed.fire {
                ctx.events.push(Event::DryFire {
                    weapon: self.inventory[cur].def,
                });
            }
            return;
        }
        let mut fired = 0;
        while self.inventory[cur].cooldown <= 0.0 && self.inventory[cur].mag > 0 && fired < 4 {
            self.fire_one(def, ctx);
            fired += 1;
            if def.fire_mode == FireMode::Semi {
                break;
            }
        }
        // MP2 reloads by itself when the magazine runs dry.
        let ws = &mut self.inventory[cur];
        if ws.mag == 0 && ws.start_reload(def) {
            ctx.events.push(Event::ReloadStarted { weapon: ws.def });
        }
    }

    fn fire_one(&mut self, def: &WeaponDef, ctx: &mut StepCtx) {
        let tu = ctx.tuning;
        let spread = def.spread_base * self.stance_spread(tu);
        let hand = self.inventory[self.current].consume_shot(def);
        let weapon = self.inventory[self.current].def;
        let origin = self.muzzle(hand, tu);
        let mut dir = (self.aim_target - origin).normalize_or_zero();
        if dir.length_sq() < 0.5 {
            dir = self.aim_dir(tu);
        }
        if self.recoil > 0.0 {
            dir = aim_dir(
                dir_yaw(dir),
                (dir_pitch(dir) + self.recoil).min(tu.pitch_max),
            );
        }
        for k in 0..def.pellets.max(1) {
            // The pump shotgun fires one straight slug plus spread pellets.
            let d = if k == 0 && def.pellets > 1 && def.class == WeaponClass::Shotgun {
                dir
            } else {
                ctx.rng.scatter(dir, spread)
            };
            let speed = def.projectile_speed
                * (1.0 + (ctx.rng.next_f32() * 2.0 - 1.0) * def.projectile_speed_random);
            ctx.shots.push(Shot {
                weapon,
                origin,
                dir: d,
                speed,
                damage: def.damage,
                impulse: def.impulse,
                range: def.range,
                hand,
            });
        }
        ctx.events.push(Event::Shot {
            weapon,
            origin,
            dir,
            hand,
        });
        self.recoil += def.recoil;
        self.since_shot = 0.0;
    }

    /// Take a painkiller: refused at full health or with none left.
    fn use_painkiller(&mut self, ctx: &mut StepCtx) {
        let max = ctx.tuning.max_health;
        if self.painkillers == 0 || self.health >= max {
            return;
        }
        let d = ctx.difficulty;
        self.painkillers -= 1;
        let now = max * d.health_gained_immediately;
        self.health = (self.health + now).min(max);
        self.heal_pending += (max * d.health_gained_from_painkiller - now).max(0.0);
        ctx.events.push(Event::Painkiller {
            health: self.health,
        });
    }

    /// Painkiller regeneration and auto-heal, on the player clock.
    fn regenerate(&mut self, ctx: &StepCtx) {
        let max = ctx.tuning.max_health;
        let d = ctx.difficulty;
        if self.heal_pending > 0.0 {
            let step = (max * d.health_regenerate_multiplier * ctx.dt).min(self.heal_pending);
            self.heal_pending -= step;
            self.health = (self.health + step).min(max);
            if self.health >= max {
                self.heal_pending = 0.0;
            }
        }
        let floor = max * d.auto_heal_level;
        if self.health < floor {
            self.health = (self.health + max * d.auto_heal_per_second * ctx.dt).min(floor);
        }
    }

    /// The capsule enemy bullets hit: (a, b, radius). Upright standing, shorter crouched or rolling,
    /// lying along the body during a dive, prone and get-up.
    pub fn hit_capsule(&self, tu: &Tuning) -> (Vec3, Vec3, f32) {
        let lying_axis = match self.state {
            State::Shootdodge { world_dir, .. } if world_dir.length_sq() > 0.0 => Some(world_dir),
            State::Shootdodge { dir, .. } | State::Prone { dir, .. } | State::GetUp { dir, .. } => {
                Some(match dir {
                    MoveDir::Forward | MoveDir::Back => yaw_dir(self.body_yaw),
                    MoveDir::Left | MoveDir::Right => yaw_right(self.body_yaw),
                })
            }
            State::Dead { .. } => Some(yaw_dir(self.body_yaw)),
            _ => None,
        };
        let r = tu.player_hit_radius;
        match lying_axis {
            Some(axis) => {
                let rr = r.min(tu.prone_top * 0.5);
                let c = self.pos + Vec3::Y * (tu.prone_top * 0.5).max(rr);
                let half = (tu.prone_hit_length * 0.5 - rr).max(0.0);
                (c - axis * half, c + axis * half, rr)
            }
            None => {
                let top = self.capsule_top(tu);
                (
                    self.pos + Vec3::Y * r,
                    self.pos + Vec3::Y * (top - r).max(r),
                    r,
                )
            }
        }
    }

    /// Damage from outside (enemy fire, explosions). `dir` is the way the hit travelled.
    /// Returns true if this killed the player.
    pub fn take_damage(
        &mut self,
        damage: f32,
        dir: Vec3,
        bt: &mut BulletTime,
        events: &mut Vec<Event>,
    ) -> bool {
        if !self.alive() || self.invulnerable() {
            return false;
        }
        self.health -= damage;
        events.push(Event::PlayerHit { damage, dir });
        if self.health <= 0.0 {
            self.health = 0.0;
            self.heal_pending = 0.0;
            self.state = State::Dead { t: 0.0 };
            if bt.set_active(false).ended {
                events.push(Event::BulletTimeEnded { ran_out: false });
            }
            bt.blocked = false;
            bt.change_game_speed(1.0, 0.5);
            events.push(Event::PlayerDied);
            return true;
        }
        {
            let from_dir = -dir;
            let x = from_dir.dot(yaw_right(self.body_yaw));
            let y = from_dir.dot(yaw_dir(self.body_yaw));
            let from = if y.abs() >= x.abs() {
                if y >= 0.0 {
                    HitSide::Front
                } else {
                    HitSide::Back
                }
            } else if x > 0.0 {
                HitSide::Right
            } else {
                HitSide::Left
            };
            self.last_hit = Some((0.0, from));
        }
        false
    }

    // --- animation --------------------------------------------------------------------------

    fn lower_clip(&self) -> Clip {
        let vh = self.vel.horizontal();
        let local = MoveDir::from_local(vh.dot(yaw_right(self.yaw)), vh.dot(yaw_dir(self.yaw)));
        let moving = vh.length() > 0.3;
        match self.state {
            State::Ground => {
                if moving {
                    Clip::Run(local)
                } else {
                    Clip::Idle
                }
            }
            State::Crouch => {
                if moving {
                    Clip::CrouchWalk(local)
                } else {
                    Clip::Crouch
                }
            }
            State::JumpStart { forward, .. } => {
                if forward {
                    Clip::JumpForward
                } else {
                    Clip::Jump
                }
            }
            State::Air { jumped } => {
                if jumped && self.vel.y > 0.0 {
                    Clip::Jump
                } else {
                    Clip::Fall
                }
            }
            State::Land { hard, .. } => {
                if hard {
                    Clip::HardLand
                } else {
                    Clip::Land
                }
            }
            State::Dodge { dir, .. } => Clip::Dodge(dir),
            State::Shootdodge { dir, .. } => Clip::Shootdodge(dir),
            State::Prone { dir, .. } => Clip::Prone(dir),
            State::GetUp { dir, .. } => Clip::GetUp(dir),
            State::Spin { .. } => Clip::Spin,
            State::Dead { .. } => Clip::Death,
        }
    }

    fn track_anim(&mut self, dt: f32) {
        let c = self.lower_clip();
        if c == self.anim_clip {
            self.anim_time += dt;
        } else {
            self.anim_clip = c;
            self.anim_time = 0.0;
        }
        // The dive's clip time is the state's own: a jump dive enters mid-clip and a dive still in
        // the air holds its last airborne frame, so elapsed time would show the wrong pose.
        if let State::Shootdodge { t, .. } = self.state {
            self.anim_time = t;
        }
    }

    /// What the skeleton should play this tick. `time_scale` is the player clock rate.
    pub fn anim(&self, weapons: &[WeaponDef], tu: &Tuning, time_scale: f32) -> AnimRequest {
        let upper = if matches!(
            self.state,
            State::Dodge { .. } | State::GetUp { .. } | State::Dead { .. }
        ) || self.inventory.is_empty()
        {
            None
        } else {
            let ws = &self.inventory[self.current];
            let class = weapons[ws.def].class;
            Some(if self.switch_timer > 0.0 {
                (Clip::Switch(class), tu.switch_time - self.switch_timer)
            } else if let Some(t) = ws.reload {
                (Clip::Reload(class), t)
            } else if self.since_shot < 0.25 {
                (Clip::Fire(class), self.since_shot)
            } else {
                (Clip::Aim(class), 0.0)
            })
        };
        AnimRequest {
            lower: self.anim_clip,
            lower_time: self.anim_time,
            upper,
            body_yaw: self.body_yaw,
            aim_yaw: wrap_angle(self.yaw - self.body_yaw),
            aim_pitch: (self.pitch + self.recoil).clamp(tu.pitch_min, tu.pitch_max),
            speed: self.vel.horizontal().length(),
            blend: 0.15,
            time_scale,
        }
    }
}
