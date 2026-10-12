//! MP2's slow-motion model (docs/behaviour/bullet-time.md), number by number, then in the sim.
mod common;
use common::*;
use payne::{BulletTime, Event, Input, Tuning, Vec3};

fn run(bt: &mut BulletTime, tu: &Tuning, secs: f32) {
    for _ in 0..(secs / DT).round() as usize {
        bt.update(DT, tu);
    }
}

#[test]
fn a_full_meter_lasts_fifteen_seconds() {
    let tu = Tuning::default();
    let mut bt = BulletTime::default();
    assert!(bt.toggle(&tu).started);
    run(&mut bt, &tu, 7.5);
    assert!((bt.meter - 0.5).abs() < 1e-3, "meter {}", bt.meter);
    run(&mut bt, &tu, 7.6);
    assert!(
        !bt.active && bt.meter < 0.002,
        "ran out (then regenerates slowly)"
    );
}

#[test]
fn speeds_follow_blend_and_efficiency() {
    let tu = Tuning::default();
    let mut bt = BulletTime::default();
    bt.toggle(&tu);
    run(&mut bt, &tu, 0.15);
    assert!(
        (bt.blend - 0.5).abs() < 0.02,
        "half blended after half of InBlendTime"
    );
    run(&mut bt, &tu, 0.2);
    assert_eq!(bt.blend, 1.0);
    // No kills: everyone at 0.6.
    assert!((bt.world_time_scale(&tu) - 0.6).abs() < 1e-5);
    assert!((bt.player_time_scale(&tu) - 0.6).abs() < 1e-5);
    // Three kills: efficiency saturates at 1 → world 0.2, player 0.2 × 3.
    for _ in 0..3 {
        bt.on_kill(0.0, &tu);
        run(&mut bt, &tu, 0.25);
    }
    assert_eq!(bt.efficiency, 1.0);
    assert!((bt.world_time_scale(&tu) - 0.2).abs() < 1e-5);
    assert!((bt.player_time_scale(&tu) - 0.6).abs() < 1e-5);
}

#[test]
fn efficiency_blends_in_holds_then_decays() {
    let tu = Tuning::default();
    let mut bt = BulletTime::default();
    bt.on_kill(0.0, &tu);
    run(&mut bt, &tu, 0.1);
    assert!(
        (bt.efficiency - 0.2).abs() < 0.01,
        "halfway through the 0.2 s blend"
    );
    run(&mut bt, &tu, 0.1);
    assert!((bt.efficiency - 0.4).abs() < 1e-4);
    run(&mut bt, &tu, 5.0);
    assert!((bt.efficiency - 0.4).abs() < 0.005, "held for 5 s");
    run(&mut bt, &tu, 1.0);
    assert!(
        (bt.efficiency - 0.25).abs() < 0.01,
        "then -0.15/s: {}",
        bt.efficiency
    );
    run(&mut bt, &tu, 3.0);
    assert_eq!(bt.efficiency, 0.0);
}

#[test]
fn meter_refills_slowly_and_from_kills_and_has_a_threshold() {
    let tu = Tuning::default();
    let mut bt = BulletTime::default();
    bt.meter = 0.0;
    run(&mut bt, &tu, 9.0);
    assert!((bt.meter - 0.1).abs() < 1e-3, "9 s of 90 s regeneration");
    bt.on_kill(0.12, &tu);
    assert!((bt.meter - 0.22).abs() < 1e-3);
    let mut low = BulletTime::default();
    low.meter = 0.05;
    let c = low.toggle(&tu);
    assert!(c.refused && !low.active, "needs more than 7%");
    // Fade-out takes OutBlendTime.
    bt.meter = 1.0;
    bt.toggle(&tu);
    run(&mut bt, &tu, 1.0);
    bt.toggle(&tu);
    run(&mut bt, &tu, 0.75);
    assert!((bt.blend - 0.5).abs() < 0.02);
}

#[test]
fn bullet_time_drains_and_a_kill_refills_it() {
    let mut t = flat();
    t.accurate();
    t.select(2); // Desert Eagle: 41 × 3 to the head
    let id = t
        .sim
        .spawn_target(Vec3::new(0.0, 0.0, -6.0), std::f32::consts::PI);
    t.tap(still(), |i| i.bullet_time = true);
    assert!(t.events.contains(&Event::BulletTimeStarted));
    t.hold(2.0, still());
    let before = t.sim.bullet_time.meter;
    assert!(
        (before - (1.0 - 2.03 / 15.0)).abs() < 0.01,
        "meter {before}"
    );
    let i = t.aim_at(still(), Vec3::new(0.0, 1.66, -6.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.until(1.0, i, |s| !s.target(id).unwrap().alive());
    let after = t.sim.bullet_time.meter;
    assert!(after > before + 0.11, "kill bonus: {before} → {after}");
    assert!(
        t.events
            .iter()
            .any(|e| matches!(e, Event::Kill { target, .. } if *target == id))
    );
}

#[test]
fn kill_adrenaline_speeds_the_player_up() {
    // Distance run in 1 real second of full bullet time with and without kill efficiency.
    let run_for = |kills: usize| {
        let mut t = flat();
        t.tap(still(), |i| i.bullet_time = true);
        t.hold(0.5, still());
        for _ in 0..kills {
            t.sim.bullet_time.on_kill(0.0, &t.sim.tuning);
        }
        t.hold(0.5, still());
        let ratio = t.sim.player_time_scale() / t.sim.world_time_scale();
        let z0 = t.sim.player.pos.z;
        let w0 = t.sim.world_time;
        t.hold(1.0, fwd());
        (
            (t.sim.player.pos.z - z0).abs(),
            (t.sim.world_time - w0) as f32,
            ratio,
        )
    };
    let (d0, w0, r0) = run_for(0);
    let (d1, w1, r1) = run_for(1);
    assert!(
        (r0 - 1.0).abs() < 1e-4 && (r1 - 1.8).abs() < 1e-3,
        "{r0} {r1}"
    );
    assert!(d1 > d0 * 1.2, "one kill: {d1} m vs {d0} m");
    // Relative to the world clock, Max moves about 1.8× as far per world second.
    let rel = (d1 / w1) / (d0 / w0);
    assert!(rel > 1.7 && rel < 2.0, "relative speed-up {rel}"); // 1.8 plus the faster run-up
}

#[test]
fn shootdodge_blocks_and_ends_bullet_time() {
    let mut t = flat();
    t.tap(still(), |i| i.bullet_time = true);
    t.hold(0.3, fwd());
    t.step(Input {
        shootdodge: true,
        ..fwd()
    });
    t.until(3.0, still(), |s| {
        matches!(
            s.player.state,
            payne::State::Shootdodge { airborne: true, .. }
        )
    });
    // Frame 12: Gm_EnableBulletTime(false) + block.
    assert!(!t.sim.bullet_time.active);
    t.tap(still(), |i| i.bullet_time = true);
    assert!(!t.sim.bullet_time.active, "toggle blocked mid-dive");
}
