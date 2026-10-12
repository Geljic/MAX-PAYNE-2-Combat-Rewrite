//! The sandbox: `cargo run --release --example range --features bevy` from the project root.
//!
//! Movement, aiming, shooting, bullet time and shootdodge against a few targets, plus a paper-target
//! gallery lane to the right (boards at 5-50 m, bullet holes, accuracy HUD; Backspace/G resets,
//! V inspects the last board hit). H shows the bindings. Combat mode (F2, or `--combat`) swaps the
//! paper targets for a wave of armed enemies in real skins; P takes a painkiller. Hands-free checks:
//!
//!     -- --script scripts/run.txt --shots /tmp/shots    feed a scripted input timeline, save the
//!                                                        screenshots it lists, then exit
//! (script format: see `payne::plugin::PayneScript`).

use bevy::prelude::*;
use payne::TestWorld;
use payne::math::{Vec3 as PVec, dir_yaw};
use payne::plugin::*;
use std::time::Duration;

/// Target feet positions (x, y, z), spawn order = the `aim=tN` index in scripts. They all face the
/// start point.
const TARGETS: [[f32; 3]; 8] = [
    [0.0, 0.0, -8.0],
    [1.8, 0.0, -4.2],
    [-2.2, 0.0, -4.8],
    [-4.5, 0.0, -9.5],
    [0.0, 0.0, -14.5],
    [-7.0, 1.2, -8.0],
    [8.0, 0.0, -16.0],
    [-6.0, 0.0, -20.0],
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| {
        args.iter()
            .position(|x| x == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let world = TestWorld::range();

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "payne range".into(),
            resolution: (1600, 900).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(PaynePlugin::default())
    .insert_resource(PayneWorld(Box::new(world.clone())))
    .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.82)))
    .insert_resource(RangeBoxes(world.boxes.clone()))
    .add_systems(Startup, (scenery, spawn_targets))
    .add_systems(Update, respawn)
    .add_systems(PostUpdate, grid);

    if args.iter().any(|a| a == "--combat") {
        app.add_systems(Startup, start_combat.after(spawn_targets));
    }
    if let Some(path) = get("--script") {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let dir = get("--shots").unwrap_or_else(|| "shots".into());
        let script = PayneScript::parse(&text, dir).unwrap_or_else(|e| panic!("{e}"));
        // one frame = one tick, so a script replays the same on any machine
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f64(1.0 / 60.0),
        ))
        .insert_resource(script)
        .add_systems(Update, (log_stats, log_enemies));
    }
    app.run();
}

#[derive(Resource)]
struct RangeBoxes(Vec<payne::Aabb>);

fn scenery(
    mut commands: Commands,
    boxes: Res<RangeBoxes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    spawn_ground(&mut commands, &mut meshes, &mut mats);
    spawn_boxes(&mut commands, &mut meshes, &mut mats, &boxes.0);
    spawn_gallery(&mut commands, &mut meshes, &mut mats);
}

fn grid(mut gizmos: Gizmos) {
    gizmos.grid(
        Isometry3d::new(
            Vec3::Y * 0.01,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        ),
        UVec2::splat(80),
        Vec2::splat(1.0),
        Color::srgba(0.7, 0.75, 0.7, 0.2),
    );
}

fn place_targets(sim: &mut payne::Sim) {
    sim.targets.clear();
    sim.corpses.clear();
    for [x, y, z] in TARGETS {
        let pos = PVec::new(x, y, z);
        sim.spawn_target(pos, dir_yaw(PVec::new(-x, 0.0, -z)));
    }
}

fn spawn_targets(mut sim: ResMut<PayneSim>) {
    place_targets(&mut sim.0);
}

/// `--combat`: start with the enemy wave instead of the paper targets.
fn start_combat(mut sim: ResMut<PayneSim>, mut combat: ResMut<PayneCombat>) {
    spawn_wave(&mut sim.0);
    combat.active = true;
}

/// T: new paper targets (leaves combat mode).
fn respawn(
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<PayneSim>,
    mut combat: ResMut<PayneCombat>,
) {
    if keys.just_pressed(KeyCode::KeyT) {
        place_targets(&mut sim.0);
        combat.active = false;
    }
}

/// Script runs: print the gallery numbers whenever the shot count changes, for checking spreads.
fn log_stats(stats: Res<GalleryStats>, sim: Res<PayneSim>, mut last: Local<(u32, u32)>) {
    if (stats.shots, stats.hits) == *last {
        return;
    }
    *last = (stats.shots, stats.hits);
    let boards: Vec<String> = stats
        .boards
        .iter()
        .map(|b| {
            format!(
                "{}h/{}pts/grp {:.1}cm/off {:.1},{:.1}",
                b.hits,
                b.score,
                b.group_cm(),
                b.offset_cm()[0],
                b.offset_cm()[1]
            )
        })
        .collect();
    println!(
        "t={:.2} pos {:.1},{:.1} shots {} hits {} | {}",
        sim.0.time,
        sim.0.player.pos.x,
        sim.0.player.pos.z,
        stats.shots,
        stats.hits,
        boards.join(" | ")
    );
}

/// Script runs: print each enemy's mode whenever it changes, to pick what to look at.
fn log_enemies(sim: Res<PayneSim>, mut last: Local<Vec<&'static str>>) {
    let now: Vec<&'static str> = sim
        .0
        .targets
        .iter()
        .map(|t| match (&t.ai, t.alive()) {
            (Some(ai), true) if ai.crouched => "crouched",
            (Some(ai), true) => ai.mode.name(),
            (Some(_), false) => "dead",
            _ => "-",
        })
        .collect();
    for (i, m) in now.iter().enumerate() {
        if last.get(i) != Some(m) {
            println!("t={:.2} enemy {i}: {m}", sim.0.time);
        }
    }
    *last = now;
}
