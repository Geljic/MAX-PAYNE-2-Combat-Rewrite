//! Moves follow their animation's root-motion curve: the dive builds speed on take-off, flies at
//! the clip's airborne speed and slides to a stop; a dive still in the air holds its last airborne
//! frame until touchdown. Real-data tests skip when `assets/generated/payne.data` is absent.
mod common;
use common::*;
use payne::{Aabb, Event, GameData, Input, Sim, State, TestWorld, Vec3};

fn real() -> Option<GameData> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/generated/payne.data");
    std::path::Path::new(path)
        .exists()
        .then(|| GameData::load(path).expect("payne.data parses"))
}

/// Dive geometry is timed on the real clock, so shootdodge slow motion is off here.
fn with(w: TestWorld, spawn: Vec3, data: Option<GameData>) -> T {
    let mut t = on(w, spawn);
    if let Some(g) = data {
        t.sim = Sim::with_data(spawn, 0.0, g);
    }
    t.sim.tuning.shootdodge_slowmo = 0.0;
    t
}

/// What one dive did, measured tick by tick.
#[derive(Debug)]
struct Dive {
    /// Horizontal distance from the dive start at take-off, at touchdown and at Prone.
    at_takeoff: f32,
    at_landing: f32,
    total: f32,
    /// Player seconds from the dive start to take-off and to touchdown.
    t_takeoff: f32,
    t_landing: f32,
    /// Horizontal speed Prone starts with.
    prone_speed: f32,
    /// `lower_time` on every tick between take-off and touchdown.
    air_lower_times: Vec<f32>,
    /// Lowest height reached and the height at Prone.
    end_y: f32,
}

/// Run forward, dive, and step with no input until Prone. The player clock is real time here
/// (no bullet time, `shootdodge_slowmo` off), so seconds × fps are clip frames.
fn dive(t: &mut T) -> Dive {
    t.hold(0.3, fwd());
    t.step(Input {
        shootdodge: true,
        ..fwd()
    });
    assert!(matches!(t.state(), State::Shootdodge { .. }), "{:?}", t.trace);
    let start = t.sim.player.pos;
    let hd = |p: Vec3| (p - start).horizontal().length();
    let (mut at_takeoff, mut at_landing, mut t_takeoff, mut t_landing) = (None, None, 0.0, 0.0);
    let mut air_lower_times = Vec::new();
    let mut time = 0.0;
    while matches!(t.state(), State::Shootdodge { .. }) {
        t.events.clear();
        t.step(still());
        time += DT;
        let airborne = matches!(
            t.state(),
            State::Shootdodge {
                airborne: true,
                landed: false,
                ..
            }
        );
        if airborne && at_takeoff.is_none() {
            at_takeoff = Some(hd(t.sim.player.pos));
            t_takeoff = time;
        }
        if airborne {
            air_lower_times.push(t.sim.anim().lower_time);
        }
        if t.events.contains(&Event::ShootdodgeLanded) {
            at_landing = Some(hd(t.sim.player.pos));
            t_landing = time;
        }
        assert!(time < 10.0, "{:?}", t.trace);
    }
    assert!(matches!(t.state(), State::Prone { .. }), "{:?}", t.trace);
    Dive {
        at_takeoff: at_takeoff.expect("took off"),
        at_landing: at_landing.expect("landed"),
        total: hd(t.sim.player.pos),
        t_takeoff,
        t_landing,
        prone_speed: t.sim.player.vel.horizontal().length(),
        air_lower_times,
        end_y: t.sim.player.pos.y,
    }
}

#[test]
fn real_forward_dive_follows_the_mov_curve() {
    let Some(g) = real() else { return };
    let fps = g.moves.shootdodge[0].fps;
    assert!(!g.moves.shootdodge[0].root.is_empty(), "run payne-extract measure first");
    let mut t = with(TestWorld::flat(), Vec3::ZERO, Some(g));
    let d = dive(&mut t);
    let air_speed = (d.at_landing - d.at_takeoff) / (d.t_landing - d.t_takeoff);
    println!(
        "real forward dive: total {:.3} m, take-off {:.3} m at frame {:.1}, landing {:.3} m at frame {:.1}, \
         air speed {:.2} m/s, prone starts at {:.3} m/s",
        d.total,
        d.at_takeoff,
        d.t_takeoff * fps,
        d.at_landing,
        d.t_landing * fps,
        air_speed,
        d.prone_speed
    );
    assert!((d.total - 5.8).abs() < 0.15, "total {}", d.total);
    assert!((d.at_landing - 4.1).abs() < 0.2, "at landing {}", d.at_landing);
    assert!((air_speed - 5.2).abs() < 0.3, "air speed {air_speed}");
    assert!((d.t_landing * fps - 27.0).abs() <= 1.0, "landing frame {}", d.t_landing * fps);
    assert!(d.prone_speed < 0.1, "Prone must not slide a second time: {}", d.prone_speed);
}

#[test]
fn real_take_off_is_slower_than_the_flight() {
    // Speed builds 1.5 → 4.9 m/s during the take-off, so the first 12 frames cover ≈1.5 m, far less
    // than 12 frames of the 5.2 m/s flight.
    let Some(g) = real() else { return };
    let mut t = with(TestWorld::flat(), Vec3::ZERO, Some(g));
    let d = dive(&mut t);
    let ground_speed = d.at_takeoff / d.t_takeoff;
    println!("real take-off: {:.3} m in {:.3} s ({ground_speed:.2} m/s)", d.at_takeoff, d.t_takeoff);
    assert!((d.at_takeoff - 1.5).abs() < 0.15, "{}", d.at_takeoff);
    assert!(ground_speed < 4.2);
}

#[test]
fn without_a_curve_the_dive_keeps_a_constant_speed() {
    // Defaults carry no root curve: distance / duration all the way, and Prone starts with that
    // speed and slides it off with `prone_slide_decel` (the old behaviour, kept as the fallback).
    let mut t = with(TestWorld::flat(), Vec3::ZERO, None);
    let sd = t.sim.moves.shootdodge[0].clone();
    assert!(sd.root.is_empty());
    let d = dive(&mut t);
    let v = sd.speed();
    assert!((d.at_takeoff / d.t_takeoff - v).abs() < 0.05, "ground {}", d.at_takeoff / d.t_takeoff);
    let air = (d.at_landing - d.at_takeoff) / (d.t_landing - d.t_takeoff);
    assert!((air - v).abs() < 0.05, "air {air}");
    assert!((d.total - sd.distance).abs() < 0.1, "total {}", d.total);
    assert!((d.prone_speed - v).abs() < 0.05, "prone {}", d.prone_speed);
}

/// A 2 m high platform reaching from behind the spawn to 2.5 m ahead of it (forward is -z).
fn ledge() -> TestWorld {
    TestWorld::flat().with_box(Aabb::new(Vec3::new(-10.0, 0.0, -2.5), Vec3::new(10.0, 2.0, 10.0)))
}

fn dive_off_ledge(data: Option<GameData>) {
    let label = if data.is_some() { "real" } else { "defaults" };
    let mut flat_t = with(TestWorld::flat(), Vec3::new(0.0, 0.0, 1.1), data.clone());
    let flat_d = dive(&mut flat_t);
    let mut t = with(ledge(), Vec3::new(0.0, 2.0, 1.1), data);
    let sd = t.sim.moves.shootdodge[0].clone();
    let tu = t.sim.tuning.clone();
    let (_, t_land) = sd.air_window(tu.gravity, tu.jump_scale).unwrap();
    let hold = t_land - 1.0 / sd.fps;
    // 0.3 s of running from z = 1.1 starts the dive at z ≈ 0, 2.5 m before the edge.
    let d = dive(&mut t);
    let held: Vec<f32> = d
        .air_lower_times
        .iter()
        .copied()
        .filter(|&lt| (lt - hold).abs() < 1e-4)
        .collect();
    println!(
        "{label} ledge dive: landed {:.2} m out (flat {:.2}), {:.2} s in the air (flat {:.2}), held the frame-{:.0} pose for {} ticks, rest at {:.2} m",
        d.at_landing,
        flat_d.at_landing,
        d.t_landing - d.t_takeoff,
        flat_d.t_landing - flat_d.t_takeoff,
        hold * sd.fps,
        held.len(),
        d.total
    );
    // Never past the hold frame in the air: no landing or lying pose mid-air.
    assert!(d.air_lower_times.iter().all(|&lt| lt <= hold + 1e-4), "{:?}", d.air_lower_times);
    assert!(held.len() >= 10, "held only {} ticks", held.len());
    assert!(d.at_landing > flat_d.at_landing + 0.8, "{} vs {}", d.at_landing, flat_d.at_landing);
    assert!(d.end_y.abs() < 1e-4, "on the lower floor");
    assert!(d.total > d.at_landing + 0.5, "slid after landing");
    // Lies down, then gets up as usual.
    assert!(t.saw("prone") || matches!(t.state(), State::Prone { .. }));
    t.step(fwd());
    t.until(3.0, still(), |s| s.player.state == State::Ground);
}

#[test]
fn diving_off_a_ledge_holds_the_flight_pose_until_touchdown() {
    dive_off_ledge(None);
}

#[test]
fn real_diving_off_a_ledge_holds_the_flight_pose_until_touchdown() {
    let Some(g) = real() else { return };
    dive_off_ledge(Some(g));
}

#[test]
fn touchdown_jumps_the_clip_to_its_landing_frame() {
    let mut t = with(ledge(), Vec3::new(0.0, 2.0, 1.1), None);
    let sd = t.sim.moves.shootdodge[0].clone();
    let tu = t.sim.tuning.clone();
    let (_, t_land) = sd.air_window(tu.gravity, tu.jump_scale).unwrap();
    t.hold(0.3, fwd());
    t.step(Input {
        shootdodge: true,
        ..fwd()
    });
    t.events.clear();
    t.until(5.0, still(), |s| {
        matches!(s.player.state, State::Shootdodge { landed: true, .. })
    });
    let lt = t.sim.anim().lower_time;
    assert!(lt >= t_land - 1e-4 && lt < t_land + 2.0 * DT, "lower_time {lt}, landing {t_land}");
}
