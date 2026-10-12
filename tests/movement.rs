//! Movement scripts with expected end states: running, strafing, jumping, landing, dodging.
mod common;
use common::*;
use payne::{Aabb, AnimRequest, Clip, Input, MoveDir, State, TestWorld, Vec3};

#[test]
fn run_reaches_speed_and_stops_short() {
    let mut t = flat();
    let tu = t.sim.tuning.clone();
    t.hold(2.0, fwd());
    let v = t.sim.player.vel.length();
    assert!((v - tu.run_speed).abs() < 0.01, "run speed {v}");
    // Yaw 0 faces -Z.
    let z0 = t.sim.player.pos.z;
    assert!(z0 < -9.5 && z0 > -10.5, "ran to {z0}");
    t.hold(1.0, still());
    let stop = (t.sim.player.pos.z - z0).abs();
    let expected = tu.run_speed * tu.run_speed / (2.0 * tu.decel);
    assert!(
        (stop - expected).abs() < 0.06,
        "stopped in {stop}, expected {expected}"
    );
    assert_eq!(t.sim.player.vel, Vec3::ZERO);
    assert_eq!(t.state(), State::Ground);
}

#[test]
fn strafing_and_backpedalling_keep_facing_the_aim() {
    let mut t = flat();
    let tu = t.sim.tuning.clone();
    t.hold(1.0, mv(1.0, 0.0));
    let p = &t.sim.player;
    assert!(p.pos.x > 3.5 && p.pos.z.abs() < 1e-4, "{:?}", p.pos);
    assert_eq!((p.yaw, p.body_yaw), (0.0, 0.0));
    assert!((p.vel.length() - tu.strafe_speed).abs() < 0.01);
    let a: AnimRequest = t.sim.anim();
    assert_eq!((a.lower, a.aim_yaw), (Clip::Run(MoveDir::Right), 0.0));

    t.hold(1.5, mv(0.0, -1.0));
    let p = &t.sim.player;
    assert!(p.pos.z > 3.0 && p.body_yaw == 0.0);
    assert!((p.vel.length() - tu.backpedal_speed).abs() < 0.01);
    assert_eq!(t.sim.anim().lower, Clip::Run(MoveDir::Back));
}

#[test]
fn moves_follow_the_aim_after_turning() {
    let mut t = flat();
    // Turn left 90° (positive yaw) in one tick of raw mouse, then run forward: -X.
    t.step(Input {
        look: [std::f32::consts::FRAC_PI_2, 0.0],
        ..Default::default()
    });
    t.hold(1.0, fwd());
    let p = &t.sim.player;
    assert!(p.pos.x < -3.5 && p.pos.z.abs() < 1e-3, "{:?}", p.pos);
    assert!((p.body_yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
}

#[test]
fn pitch_is_clamped() {
    let mut t = flat();
    t.hold(
        0.5,
        Input {
            look: [0.0, 1.0],
            ..Default::default()
        },
    );
    assert_eq!(t.sim.player.pitch, t.sim.tuning.pitch_max);
}

#[test]
fn standing_jump_follows_its_arc_and_lands() {
    let mut t = flat();
    let tu = t.sim.tuning.clone();
    let v = 7.5 * tu.jump_scale; // Jump_Up: C_Jump(7.5) at frame 9
    t.tap(still(), |i| i.jump = true);
    let windup = t.until(1.0, still(), |s| {
        matches!(s.player.state, State::Air { .. })
    });
    assert!(
        (windup + 2.0 * DT - 9.0 / 30.0).abs() <= DT * 1.5,
        "wind-up {windup}"
    );
    let mut apex: f32 = 0.0;
    let mut air = 0;
    while matches!(t.state(), State::Air { .. }) {
        t.step(still());
        apex = apex.max(t.sim.player.pos.y);
        air += 1;
        assert!(air < 600);
    }
    let want = v * v / (2.0 * tu.gravity);
    assert!((apex - want).abs() / want < 0.03, "apex {apex} want {want}");
    let airtime = air as f32 * DT;
    assert!(
        (airtime - 2.0 * v / tu.gravity).abs() < 2.0 * DT,
        "airtime {airtime}"
    );
    assert!(t.sim.player.pos.y.abs() < 1e-5);
    assert!(t.count(|e| matches!(e, payne::Event::Landed { .. })) == 1);
    t.until(1.0, still(), |s| s.player.state == State::Ground);
}

#[test]
fn running_jump_keeps_momentum() {
    let mut t = flat();
    t.hold(1.0, fwd());
    let z0 = t.sim.player.pos.z;
    t.tap(fwd(), |i| i.jump = true);
    t.until(3.0, fwd(), |s| matches!(s.player.state, State::Land { .. }));
    let dist = (t.sim.player.pos.z - z0).abs();
    // Run speed × airtime: 5.25 m/s × (2 × 7.5 / 20 = 0.75 s) ≈ 3.9 m, plus a little wind-up.
    let tu = &t.sim.tuning;
    let want = tu.run_speed * 2.0 * 7.5 * tu.jump_scale / tu.gravity;
    assert!(dist > want * 0.9 && dist < want * 1.3, "jumped {dist}, want about {want}");
}

#[test]
fn walking_off_a_ledge_falls_and_hard_lands_from_high() {
    let w = TestWorld::flat().with_box(Aabb::new(
        Vec3::new(-2.0, 0.0, -2.0),
        Vec3::new(2.0, 4.0, 2.0),
    ));
    let mut t = on(w, Vec3::new(0.0, 4.0, 0.0));
    t.step(still());
    assert_eq!(t.sim.player.pos.y, 4.0);
    t.until(3.0, fwd(), |s| matches!(s.player.state, State::Land { .. }));
    assert!(t.saw("air"));
    assert!(matches!(t.state(), State::Land { hard: true, .. }));
    assert!(t.sim.player.pos.y.abs() < 1e-5);
    assert_eq!(t.sim.player.vel, Vec3::ZERO);
}

#[test]
fn walls_stop_the_capsule() {
    let w = TestWorld::flat().with_box(Aabb::block(0.0, -5.0, 4.0, 3.0, 1.0));
    let mut t = on(w, Vec3::ZERO);
    t.hold(3.0, fwd());
    let r = t.sim.tuning.capsule_radius;
    let z = t.sim.player.pos.z;
    assert!((z - (-4.5 + r)).abs() < 0.02, "stopped at {z}");
}

#[test]
fn steps_up_low_boxes() {
    let w = TestWorld::flat().with_box(Aabb::block(0.0, -3.0, 4.0, 0.3, 4.0));
    let mut t = on(w, Vec3::ZERO);
    t.hold(1.0, fwd());
    assert_eq!(t.sim.player.pos.y, 0.3);
}

#[test]
fn crouch_toggles() {
    let mut t = flat();
    t.tap(still(), |i| i.crouch = true);
    assert_eq!(t.state(), State::Crouch);
    t.hold(1.0, fwd());
    let v = t.sim.player.vel.length();
    assert!((v - t.sim.tuning.crouch_speed).abs() < 0.01);
    t.tap(still(), |i| i.crouch = true);
    assert_eq!(t.state(), State::Ground);
}

#[test]
fn crouch_while_running_dodges_its_distance() {
    let mut t = flat();
    let d = t.sim.moves.dodge[0].clone();
    t.hold(0.5, fwd());
    let z0 = t.sim.player.pos.z;
    t.step(Input {
        crouch: true,
        ..fwd()
    });
    assert!(matches!(
        t.state(),
        State::Dodge {
            dir: MoveDir::Forward,
            ..
        }
    ));
    let secs = t.until(3.0, fwd(), |s| {
        !matches!(s.player.state, State::Dodge { .. })
    }) + DT;
    let dist = (t.sim.player.pos.z - z0).abs();
    assert!((dist - d.distance).abs() < 0.15, "dodged {dist}");
    assert!((secs - d.duration()).abs() < 2.0 * DT, "took {secs}");
    // C_Jump(4) at frame 11: the roll leaves the ground.
    assert!(t.sim.player.pos.y.abs() < 1e-5);
}

#[test]
fn side_dodge_slows_the_game_by_its_messages() {
    let mut t = flat();
    t.hold(0.5, mv(1.0, 0.0));
    t.step(Input {
        crouch: true,
        ..mv(1.0, 0.0)
    });
    assert!(matches!(
        t.state(),
        State::Dodge {
            dir: MoveDir::Right,
            ..
        }
    ));
    t.step(mv(1.0, 0.0)); // the timeline's first step plays frame 0
    // Player_Dodge_Right: Gm_ChangeGameSpeed(0.7, 0) at frame 0, back to 1.0 over 0.5 s at 25.
    assert!((t.sim.world_time_scale() - 0.7).abs() < 1e-5);
    assert!(t.sim.player.invulnerable());
    t.until(5.0, mv(1.0, 0.0), |s| s.world_time_scale() >= 1.0);
    assert!(!t.sim.player.invulnerable());
}
