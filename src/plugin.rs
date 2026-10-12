//! Bevy integration (feature `bevy`): add [`PaynePlugin`], optionally replace the [`PayneWorld`],
//! and you have Max Payne 2 style third-person gunplay with a capsule rig driven by the real clips.
//!
//! - The sim runs in `FixedUpdate` at 60 Hz and keeps its own clocks: bullet time never touches
//!   Bevy's `Time<Virtual>`.
//! - Input ([`PayneInput`]): keyboard + mouse and gamepad. Buttons are held booleans; presses are
//!   latched between fixed ticks so a quick tap is never lost. Look deltas are raw radians.
//! - Camera: over the right shoulder, following the aim, pulling in against walls; the crosshair
//!   ray through the screen centre goes to the sim as `Input::camera_ray`.
//! - Look: [`PayneConfig`] names `payne.data` and `anims.bin` (both optional; without them the
//!   defaults and a procedural capsule pose are used).
//! - Enemies: targets with a brain are posed from their [`crate::enemy::EnemyAnim`] (clips found
//!   through `clip_candidates`, gun and aim like Max's), shoot visible tracers, and hurt Max with
//!   a red edge flash, a hit-direction mark and the painkiller HUD. [`PayneCombat`] / [`spawn_wave`]
//!   put a wave of them in the range.
//! - Scripts ([`PayneScript`]): a timed input list that replaces the player and takes screenshots,
//!   for visual checks without hands.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use std::path::PathBuf;

use crate::clips::AnimSet;
use crate::data::GameData;
use crate::event::Event;
use crate::math::Vec3 as PVec;
use crate::player::Input;
use crate::sim::Sim;
use crate::world::{TestWorld, World};

mod camera;
mod combat;
mod gallery;
mod hud;
mod pose;
mod rig;
mod script;

pub use camera::{CameraRig, PayneCamera};
pub use combat::{PayneCombat, spawn_wave};
pub use gallery::{BoardStats, GalleryStats, GalleryView, spawn_gallery};
pub use rig::{spawn_boxes, spawn_ground};
pub use script::PayneScript;

/// Convert the core's vector to Bevy's.
pub fn bv(v: PVec) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// Convert Bevy's vector to the core's.
pub fn pv(v: Vec3) -> PVec {
    PVec::new(v.x, v.y, v.z)
}

/// Where things come from and where the player starts.
#[derive(Clone, Debug)]
pub struct PayneConfig {
    /// `payne.data` (missing file: defaults).
    pub data: PathBuf,
    /// `anims.bin` (missing file: procedural capsule pose).
    pub anims: PathBuf,
    pub spawn: PVec,
    pub spawn_yaw: f32,
    /// Spawn the camera, HUD and lights. Off when the host has its own.
    pub camera: bool,
    pub hud: bool,
}

impl Default for PayneConfig {
    fn default() -> Self {
        PayneConfig {
            data: "assets/generated/payne.data".into(),
            anims: "assets/generated/anims.bin".into(),
            spawn: PVec::ZERO,
            spawn_yaw: 0.0,
            camera: true,
            hud: true,
        }
    }
}

#[derive(Default)]
pub struct PaynePlugin {
    pub config: PayneConfig,
}

/// The simulation.
#[derive(Resource)]
pub struct PayneSim(pub Sim);

/// The level. Replace it with your own collision.
#[derive(Resource)]
pub struct PayneWorld(pub Box<dyn World + Send + Sync>);

impl Default for PayneWorld {
    fn default() -> Self {
        PayneWorld(Box::new(TestWorld::range()))
    }
}

/// Loaded animations, if the file was there.
#[derive(Resource, Default)]
pub struct PayneAnims(pub Option<AnimSet>);

/// Everything the sim reported, forwarded each fixed tick (hook sounds and effects here).
#[derive(Message, Clone, Debug)]
pub struct PayneEvent(pub Event);

/// Overlay toggles.
#[derive(Resource, Default)]
pub struct PayneUi {
    pub help: bool,
    /// Debug gizmos: collider, aim ray, aim point, hit zones.
    pub debug: bool,
}

/// Presses seen since the last fixed tick.
#[derive(Clone, Copy, Default)]
struct Latch {
    fire: bool,
    reload: bool,
    jump: bool,
    crouch: bool,
    bullet_time: bool,
    shootdodge: bool,
    painkiller: bool,
    next: bool,
    prev: bool,
    select: Option<usize>,
}

/// What the player is asking for. Written every frame by [`gather_input`] (or the script) and
/// consumed by the fixed tick.
#[derive(Resource, Default)]
pub struct PayneInput {
    /// Held state this frame (`look` and `camera_ray` are filled at the tick).
    pub held: Input,
    latch: Latch,
    /// Look not yet consumed by a tick: (yaw, pitch) radians. The camera adds it so the view
    /// follows the mouse at frame rate.
    pub look: [f32; 2],
}

impl PayneInput {
    /// The input for one tick: held buttons plus latched taps, all pending look.
    fn take(&mut self, camera_ray: Option<(PVec, PVec)>) -> Input {
        let l = std::mem::take(&mut self.latch);
        let mut i = self.held;
        i.fire |= l.fire;
        i.reload |= l.reload;
        i.jump |= l.jump;
        i.crouch |= l.crouch;
        i.bullet_time |= l.bullet_time;
        i.shootdodge |= l.shootdodge;
        i.painkiller |= l.painkiller;
        i.next_weapon |= l.next;
        i.prev_weapon |= l.prev;
        i.select_weapon = l.select.or(i.select_weapon);
        i.look = std::mem::take(&mut self.look);
        i.camera_ray = camera_ray;
        i
    }
}

impl Plugin for PaynePlugin {
    fn build(&self, app: &mut App) {
        let c = &self.config;
        let data = GameData::load(&c.data).unwrap_or_else(|e| {
            warn!("{}: {e}; using defaults", c.data.display());
            GameData::default()
        });
        let anims = match std::fs::read(&c.anims) {
            Ok(bytes) => match AnimSet::load(&bytes) {
                Ok(a) => Some(a),
                Err(e) => {
                    warn!("{e}; using the procedural pose");
                    None
                }
            },
            Err(_) => {
                info!("no {}; using the procedural pose", c.anims.display());
                None
            }
        };
        let rig_map = anims.as_ref().and_then(pose::RigMap::new);
        let mut sim = Sim::with_data(c.spawn, c.spawn_yaw, data);
        if let Some(set) = &anims {
            // Enemy run and walk speeds come from the clips' root motion at their real frame rate.
            sim.enemies.measure_speeds(set);
        }
        app.insert_resource(PayneSim(sim))
            .insert_resource(PayneAnims(anims))
            .insert_resource(rig::PayneRigMap(rig_map))
            .init_resource::<PayneInput>()
            .init_resource::<PayneUi>()
            .insert_resource(PayneCombat {
                spawn: c.spawn,
                spawn_yaw: c.spawn_yaw,
                ..default()
            })
            .init_resource::<hud::DamageFx>()
            .init_resource::<CameraRig>()
            .init_resource::<rig::GunPoints>()
            .init_resource::<rig::Streaks>()
            .add_message::<PayneEvent>()
            // Bullet time is the sim's business; Virtual time keeps its normal pace.
            .insert_resource(Time::<Fixed>::from_hz(60.0))
            .add_systems(Startup, rig::setup)
            .add_systems(
                Update,
                (
                    (gather_input, grab_cursor, toggles, combat::keys).chain(),
                    rig::sync_figures,
                    rig::handle_events,
                    rig::fx_tick,
                    script::shots,
                ),
            )
            .add_systems(FixedUpdate, (script::drive, step_sim).chain())
            .add_systems(
                PostUpdate,
                (
                    camera::follow,
                    (
                        rig::pose_player,
                        rig::pose_targets,
                        rig::pose_corpses,
                        rig::draw_figures,
                        rig::draw_streaks,
                    )
                        .chain(),
                    rig::debug_draw,
                )
                    .before(TransformSystems::Propagate),
            );
        if app.world().get_resource::<PayneWorld>().is_none() {
            app.init_resource::<PayneWorld>();
        }
        gallery::build(app, c.hud);
        if c.camera {
            app.add_systems(Startup, camera::spawn);
        }
        if c.hud {
            app.add_systems(Startup, hud::setup).add_systems(
                Update,
                (
                    hud::update,
                    hud::damage_events,
                    hud::damage_draw,
                    hud::enemy_labels,
                ),
            );
        }
    }
}

fn step_sim(
    mut sim: ResMut<PayneSim>,
    world: Res<PayneWorld>,
    mut input: ResMut<PayneInput>,
    cam: Res<CameraRig>,
    combat: Res<PayneCombat>,
    mut events: MessageWriter<PayneEvent>,
) {
    if combat.god {
        // Animation messages switch immortality off again; keep it on every tick.
        sim.0.player.immortal = true;
    }
    let ray = cam.ray();
    let i = input.take(ray);
    sim.0.step(world.0.as_ref(), &i);
    for e in sim.0.drain_events() {
        events.write(PayneEvent(e));
    }
}

/// Radians of look per pixel of mouse travel. ESTIMATE: MP2's default mouse sensitivity.
const MOUSE_SENS: f32 = 0.0022;
/// Radians per second at full right-stick deflection.
const STICK_LOOK: f32 = 2.8;

#[allow(clippy::too_many_arguments)]
fn gather_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    pads: Query<&Gamepad>,
    cursor: Single<&CursorOptions>,
    script: Option<Res<PayneScript>>,
    mut input: ResMut<PayneInput>,
) {
    if script.is_some() {
        return;
    }
    // A click that captures the mouse is not also a shot.
    let grabbed = cursor.grab_mode != CursorGrabMode::None;
    let key = |k: KeyCode| keys.pressed(k);
    let tap = |k: KeyCode| keys.just_pressed(k);
    let any = |a: KeyCode, b: KeyCode| key(a) || key(b);

    let mut mv = [
        key(KeyCode::KeyD) as i32 as f32 - key(KeyCode::KeyA) as i32 as f32,
        key(KeyCode::KeyW) as i32 as f32 - key(KeyCode::KeyS) as i32 as f32,
    ];
    let mut h = Input {
        fire: grabbed && mouse.pressed(MouseButton::Left),
        reload: key(KeyCode::KeyR),
        jump: key(KeyCode::Space),
        crouch: any(KeyCode::ControlLeft, KeyCode::KeyC),
        bullet_time: grabbed && mouse.pressed(MouseButton::Right),
        shootdodge: any(KeyCode::ShiftLeft, KeyCode::ShiftRight),
        painkiller: key(KeyCode::KeyP),
        ..default()
    };
    let mut l = Latch {
        fire: grabbed && mouse.just_pressed(MouseButton::Left),
        reload: tap(KeyCode::KeyR),
        jump: tap(KeyCode::Space),
        crouch: tap(KeyCode::ControlLeft) || tap(KeyCode::KeyC),
        bullet_time: grabbed && mouse.just_pressed(MouseButton::Right),
        shootdodge: tap(KeyCode::ShiftLeft) || tap(KeyCode::ShiftRight),
        painkiller: tap(KeyCode::KeyP),
        ..default()
    };
    if scroll.delta.y > 0.0 {
        l.next = true;
    } else if scroll.delta.y < 0.0 {
        l.prev = true;
    }
    const DIGITS: [KeyCode; 7] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
    ];
    for (n, k) in DIGITS.iter().enumerate() {
        if tap(*k) {
            l.select = Some(n);
        }
    }

    let mut look = [0.0, 0.0];
    if grabbed {
        look[0] -= motion.delta.x * MOUSE_SENS;
        look[1] -= motion.delta.y * MOUSE_SENS;
    }
    for pad in &pads {
        let s = pad.left_stick();
        if s.length() > 0.2 {
            mv = [s.x, s.y];
        }
        let r = pad.right_stick();
        if r.length() > 0.15 {
            look[0] -= r.x * STICK_LOOK * time.delta_secs();
            look[1] += r.y * STICK_LOOK * time.delta_secs();
        }
        h.fire |= pad.pressed(GamepadButton::RightTrigger2);
        h.bullet_time |= pad.pressed(GamepadButton::LeftTrigger2);
        h.jump |= pad.pressed(GamepadButton::South);
        h.crouch |= pad.pressed(GamepadButton::East);
        h.reload |= pad.pressed(GamepadButton::West);
        h.shootdodge |= pad.pressed(GamepadButton::RightTrigger);
        h.painkiller |= pad.pressed(GamepadButton::North);
        l.fire |= pad.just_pressed(GamepadButton::RightTrigger2);
        l.bullet_time |= pad.just_pressed(GamepadButton::LeftTrigger2);
        l.jump |= pad.just_pressed(GamepadButton::South);
        l.crouch |= pad.just_pressed(GamepadButton::East);
        l.reload |= pad.just_pressed(GamepadButton::West);
        l.shootdodge |= pad.just_pressed(GamepadButton::RightTrigger);
        l.painkiller |= pad.just_pressed(GamepadButton::North);
        l.next |= pad.just_pressed(GamepadButton::DPadRight);
        l.prev |= pad.just_pressed(GamepadButton::DPadLeft);
    }
    let len = (mv[0] * mv[0] + mv[1] * mv[1]).sqrt();
    if len > 1.0 {
        mv = [mv[0] / len, mv[1] / len];
    }
    h.move_axis = mv;
    input.held = h;
    let old = input.latch;
    input.latch = Latch {
        fire: old.fire | l.fire,
        reload: old.reload | l.reload,
        jump: old.jump | l.jump,
        crouch: old.crouch | l.crouch,
        bullet_time: old.bullet_time | l.bullet_time,
        shootdodge: old.shootdodge | l.shootdodge,
        painkiller: old.painkiller | l.painkiller,
        next: old.next | l.next,
        prev: old.prev | l.prev,
        select: l.select.or(old.select),
    };
    input.look[0] += look[0];
    input.look[1] += look[1];
}

fn grab_cursor(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cursor: Single<&mut CursorOptions>,
    script: Option<Res<PayneScript>>,
) {
    if script.is_some() {
        return;
    }
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

fn toggles(keys: Res<ButtonInput<KeyCode>>, mut ui: ResMut<PayneUi>, mut sim: ResMut<PayneSim>) {
    // B: whether a shootdodge slows the world on its own (MP2) or only bullet time does (default).
    if keys.just_pressed(KeyCode::KeyB) {
        let t = &mut sim.0.tuning;
        t.shootdodge_slowmo = if t.shootdodge_slowmo > 0.5 { 0.0 } else { 1.0 };
    }
    if keys.just_pressed(KeyCode::KeyH) {
        ui.help = !ui.help;
    }
    if keys.just_pressed(KeyCode::F1) {
        ui.debug = !ui.debug;
    }
}
