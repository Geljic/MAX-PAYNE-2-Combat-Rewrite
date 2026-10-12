//! Runs the sim on data generated from the player's own game files. Skips when
//! `assets/generated/payne.data` hasn't been generated (it is never committed).
use payne::{GameData, Input, Sim, State, TestWorld, Vec3};

fn real() -> Option<GameData> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/generated/payne.data");
    std::path::Path::new(path).exists().then(|| GameData::load(path).expect("payne.data parses"))
}

#[test]
fn real_data_loads_and_matches_the_game() {
    let Some(g) = real() else { return };
    let names: Vec<_> = g.weapons.iter().map(|w| w.name.as_str()).collect();
    println!("weapons {names:?}\nunknown keys: {}", g.unknown.len());
    for u in g.unknown.iter().take(30) {
        println!("  unknown: {u}");
    }
    let beretta = g.weapons.iter().find(|w| w.name == "Beretta").expect("Beretta");
    assert_eq!(beretta.magazine, 16);
    assert!((g.tuning.capsule_radius - 0.48).abs() < 1e-4);
}

#[test]
fn real_data_shootdodge_runs_its_timeline() {
    let Some(g) = real() else { return };
    let w = TestWorld::flat();
    let mut sim = Sim::with_data(Vec3::ZERO, 0.0, g);
    sim.tuning.shootdodge_slowmo = 1.0; // check MP2's own timeline
    let fwd = Input { move_axis: [0.0, 1.0], ..Default::default() };
    for _ in 0..18 {
        sim.step(&w, &fwd);
    }
    sim.step(&w, &Input { shootdodge: true, ..fwd });
    assert!(matches!(sim.player.state, State::Shootdodge { .. }), "{:?}", sim.player.state.name());
    let start = sim.player.pos;
    let mut min_speed = 1.0f32;
    let mut ticks = 0;
    while matches!(sim.player.state, State::Shootdodge { .. }) && ticks < 60 * 20 {
        sim.step(&w, &fwd);
        min_speed = min_speed.min(sim.world_time_scale());
        ticks += 1;
    }
    let dist = (sim.player.pos - start).length();
    println!("shootdodge: {ticks} real ticks, {dist:.2} m, min world speed {min_speed:.2}, ended in {}", sim.player.state.name());
    assert!(min_speed < 0.25, "slow-mo reached");
    assert!((4.5..7.5).contains(&dist), "dive distance {dist}");
}

#[test]
fn real_data_reload_fills_the_magazine() {
    let Some(g) = real() else { return };
    for name in ["Beretta", "MP5", "Kalashnikov", "DesertEagle"] {
        let w = g.weapons.iter().find(|w| w.name == name).expect(name);
        assert_eq!(w.reload_amount, w.magazine, "{name} reloads a full magazine");
    }
    let pump = g.weapons.iter().find(|w| w.name == "PumpShotgun").expect("PumpShotgun");
    assert_eq!(pump.reload_amount, 1, "the pump shotgun loads shells one at a time");
}
