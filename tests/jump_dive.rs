//! The jump dive: shootdodge pressed in the air enters the dive clip at its `C_Jump` frame, keeps
//! the vertical speed, flies at the clip's airborne speed, holds the flight pose while falling and
//! lands into the slide and Prone. Payne's own move (Remedy shipped no HIGHSHOOTDODGE animations).
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
fn sim_on(data: Option<GameData>) -> T {
    let mut t = flat();
    if let Some(g) = data {
        t.sim = Sim::with_data(Vec3::ZERO, 0.0, g);
    }
    t.sim.tuning.shootdodge_slowmo = 0.0;
    t
}

fn dive_press(base: Input) -> Input {
    Input {
        shootdodge: true,
        ..base
    }
}

/// Jump (standing or running with `base`), wait for the apex and dive there.
/// Returns (player seconds from take-off to the dive, position at take-off).
fn jump_and_dive_at_apex(t: &mut T, base: Input) -> (f32, Vec3) {
    t.tap(base, |i| i.jump = true);
    t.until(2.0, base, |s| matches!(s.player.state, State::Air { .. }));
    let take_off = t.sim.player.pos;
    let up = t.until(2.0, base, |s| s.player.vel.y <= 0.0);
    t.step(dive_press(base));
    (up, take_off)
}

#[derive(Debug)]
struct Flight {
    airtime: f32,
    air_distance: f32,
    landing_lower_time: f32,
}

/// Step with no input until the dive lands; airtime and horizontal distance from `take_off`.
fn fly(t: &mut T, take_off: Vec3, already: f32) -> Flight {
    let mut time = already;
    loop {
        t.events.clear();
        t.step(still());
        time += DT;
        if t.events.contains(&Event::ShootdodgeLanded) {
            break;
        }
        assert!(time < 5.0, "{:?}", t.trace);
    }
    Flight {
        airtime: time,
        air_distance: (t.sim.player.pos - take_off).horizontal().length(),
        landing_lower_time: t.sim.anim().lower_time,
    }
}

/// The ground dive's flight, for comparison.
fn ground_flight(data: Option<GameData>) -> Flight {
    let mut t = sim_on(data);
    t.hold(0.3, fwd());
    t.step(dive_press(fwd()));
    t.until(2.0, still(), |s| {
        matches!(s.player.state, State::Shootdodge { airborne: true, .. })
    });
    let p = t.sim.player.pos;
    fly(&mut t, p, DT)
}

fn standing_jump_dive(data: Option<GameData>) {
    let label = if data.is_some() { "real" } else { "defaults" };
    let ground = ground_flight(data.clone());
    let mut t = sim_on(data);
    t.select(0);
    for _ in 0..3 {
        t.tap(still(), |i| i.fire = true);
        t.hold(0.3, still());
    }
    let mag = t.sim.player.weapon().mag;
    let sd = t.sim.moves.shootdodge[0].clone();
    let tu = t.sim.tuning.clone();
    let (t_jump, t_land) = sd.air_window(tu.gravity, tu.jump_scale).unwrap();

    let (up, take_off) = jump_and_dive_at_apex(&mut t, still());
    let p = t.sim.player.clone();
    let State::Shootdodge {
        t: clip_t,
        airborne: true,
        landed: false,
        ..
    } = p.state
    else {
        panic!("no jump dive: {:?}", p.state)
    };
    assert!((clip_t - t_jump).abs() < DT, "entered at frame {}", clip_t * sd.fps);
    assert!(
        (t.sim.anim().lower_time - clip_t).abs() < 1e-5,
        "the pose starts at the C_Jump frame"
    );
    assert!(p.weapon().mag > mag, "frame-0 C_Reload ran ({mag} → {})", p.weapon().mag);
    assert!(p.vel.y <= 0.0, "no second C_Jump: vy {}", p.vel.y);
    // Frame 12 blocks bullet time only with shootdodge slow motion on (off here).
    assert!(!t.sim.bullet_time.blocked);
    let air_speed = sd.air_speed(tu.gravity, tu.jump_scale).unwrap();
    assert!((p.vel.horizontal().length() - air_speed).abs() < 0.05);

    // A second press in the same airtime does nothing.
    t.step(still());
    t.step(dive_press(still()));
    let State::Shootdodge { t: later, .. } = t.state() else { panic!() };
    assert!(later > clip_t, "the dive carried on, not restarted");

    let f = fly(&mut t, take_off, up + 3.0 * DT);
    println!(
        "{label} standing jump dive: {:.2} s from take-off to touchdown (ground dive flight {:.2} s), \
         {:.2} m in the air (ground dive {:.2} m), landing pose at frame {:.1}",
        f.airtime,
        ground.airtime,
        f.air_distance,
        ground.air_distance,
        f.landing_lower_time * sd.fps
    );
    assert!(f.airtime > ground.airtime + 0.1, "{} vs {}", f.airtime, ground.airtime);
    assert!(f.landing_lower_time >= t_land - 1e-4, "slides from the landing frame");
    t.until(3.0, still(), |s| matches!(s.player.state, State::Prone { .. }));
    assert!(t.sim.player.pos.y.abs() < 1e-4);
    t.step(fwd());
    assert!(matches!(t.state(), State::GetUp { .. }));
    t.until(3.0, still(), |s| s.player.state == State::Ground);
    assert!(t.events.contains(&Event::GotUp));
}

#[test]
fn standing_jump_dive_at_the_apex() {
    standing_jump_dive(None);
}

#[test]
fn real_standing_jump_dive_at_the_apex() {
    let Some(g) = real() else { return };
    standing_jump_dive(Some(g));
}

fn running_jump_dive(data: Option<GameData>) {
    let label = if data.is_some() { "real" } else { "defaults" };
    let ground = ground_flight(data.clone());
    let mut t = sim_on(data);
    t.hold(0.5, fwd());
    let (up, take_off) = jump_and_dive_at_apex(&mut t, fwd());
    assert!(matches!(t.state(), State::Shootdodge { airborne: true, .. }));
    let f = fly(&mut t, take_off, up + DT);
    println!(
        "{label} running jump dive: {:.2} s and {:.2} m from take-off to touchdown (ground dive {:.2} s, {:.2} m)",
        f.airtime, f.air_distance, ground.airtime, ground.air_distance
    );
    assert!(f.airtime > ground.airtime + 0.1);
    assert!(f.air_distance > ground.air_distance + 0.5);
    t.until(3.0, still(), |s| matches!(s.player.state, State::Prone { .. }));
}

#[test]
fn running_jump_dive_flies_further_than_a_ground_dive() {
    running_jump_dive(None);
}

#[test]
fn real_running_jump_dive_flies_further_than_a_ground_dive() {
    let Some(g) = real() else { return };
    running_jump_dive(Some(g));
}

#[test]
fn jump_dive_follows_the_stick() {
    let mut t = sim_on(None);
    t.tap(still(), |i| i.jump = true);
    t.until(2.0, still(), |s| matches!(s.player.state, State::Air { .. }));
    t.step(dive_press(mv(-1.0, 0.0)));
    assert!(matches!(
        t.state(),
        State::Shootdodge {
            dir: payne::MoveDir::Left,
            airborne: true,
            ..
        }
    ));
    t.until(5.0, still(), |s| matches!(s.player.state, State::Prone { .. }));
    assert!(t.sim.player.pos.x < -2.0 && t.sim.player.pos.z.abs() < 1e-3);
}

#[test]
fn jump_dive_option_off_does_nothing_in_the_air() {
    let mut t = flat();
    t.sim.tuning.jump_dive = 0.0;
    t.tap(still(), |i| i.jump = true);
    t.until(2.0, still(), |s| matches!(s.player.state, State::Air { .. }));
    t.step(dive_press(still()));
    assert!(matches!(t.state(), State::Air { .. }));
    t.until(2.0, still(), |s| matches!(s.player.state, State::Land { .. }));
    assert!(!t.saw("shootdodge"));
}

#[test]
fn jump_dive_boost_adds_upward_speed() {
    let mut t = flat();
    t.sim.tuning.jump_dive_boost = 3.0;
    t.tap(still(), |i| i.jump = true);
    t.until(2.0, still(), |s| matches!(s.player.state, State::Air { .. }));
    t.until(2.0, still(), |s| s.player.vel.y <= 0.0);
    let vy = t.sim.player.vel.y;
    t.step(dive_press(still()));
    // The entry tick only sets the dive up (like a ground dive's first tick): no gravity yet.
    let want = vy + 3.0;
    assert!((t.sim.player.vel.y - want).abs() < 1e-3, "{} vs {want}", t.sim.player.vel.y);
}

#[test]
fn jump_dive_with_slowmo_slows_the_world_at_once() {
    let mut t = flat();
    t.sim.tuning.shootdodge_slowmo = 1.0;
    t.tap(still(), |i| i.jump = true);
    t.until(2.0, still(), |s| matches!(s.player.state, State::Air { .. }));
    t.step(dive_press(still()));
    assert!(matches!(t.state(), State::Shootdodge { slowmo: true, .. }));
    // The frame-10 Gm_ChangeGameSpeed(0.2, 0.3) was skipped by entering at frame 12, so it runs
    // at entry: the world blends to 0.2 over 0.3 real seconds.
    t.hold(0.3, still());
    assert!(t.sim.world_time_scale() < 0.25, "{}", t.sim.world_time_scale());
}

#[test]
fn falling_off_a_ledge_can_jump_dive() {
    // Walk off a 3 m platform, then dive while falling.
    let w = TestWorld::flat().with_box(Aabb::new(Vec3::new(-5.0, 0.0, -1.0), Vec3::new(5.0, 3.0, 5.0)));
    let mut t = on(w, Vec3::new(0.0, 3.0, 0.0));
    t.until(2.0, fwd(), |s| matches!(s.player.state, State::Air { jumped: false }));
    t.hold(0.1, fwd());
    t.step(dive_press(fwd()));
    assert!(matches!(t.state(), State::Shootdodge { airborne: true, .. }));
    t.until(5.0, still(), |s| matches!(s.player.state, State::Prone { .. }));
    assert!(t.sim.player.pos.y.abs() < 1e-4);
}

#[test]
fn no_jump_dive_out_of_a_dodge_rolls_hop() {
    // A dodge roll that carries off a 3 m ledge ends in a fall; the roll had its own window to
    // become a dive (before `can_shootdodge_frame`), so the fall that follows cannot.
    let w = TestWorld::flat().with_box(Aabb::new(Vec3::new(-5.0, 0.0, -2.2), Vec3::new(5.0, 3.0, 5.0)));
    let mut t = on(w, Vec3::new(0.0, 3.0, 1.0));
    t.hold(0.2, fwd());
    t.step(Input {
        crouch: true,
        ..fwd()
    });
    assert!(matches!(t.state(), State::Dodge { .. }), "{:?}", t.trace);
    t.until(3.0, still(), |s| !matches!(s.player.state, State::Dodge { .. }));
    assert!(
        matches!(t.state(), State::Air { .. }),
        "the roll should end in the air: {:?}",
        t.trace
    );
    t.step(dive_press(fwd()));
    assert!(matches!(t.state(), State::Air { .. }), "{:?}", t.state());
    t.until(3.0, still(), |s| !matches!(s.player.state, State::Air { .. }));
    // On the ground again, diving works.
    t.until(2.0, still(), |s| s.player.state == State::Ground);
    t.step(dive_press(fwd()));
    assert!(matches!(t.state(), State::Shootdodge { .. }));
}

#[test]
fn shootdodge_during_the_jump_wind_up_is_a_ground_dive() {
    let mut t = flat();
    t.step(Input {
        jump: true,
        ..still()
    });
    assert!(matches!(t.state(), State::JumpStart { .. }));
    t.step(dive_press(Input {
        jump: true,
        ..still()
    }));
    assert!(matches!(
        t.state(),
        State::Shootdodge {
            t: 0.0,
            airborne: false,
            ..
        }
    ));
}
