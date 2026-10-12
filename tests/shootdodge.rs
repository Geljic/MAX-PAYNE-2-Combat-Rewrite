//! Shootdodge, prone and spin scripts, timed against the move timelines.
mod common;
use common::*;
use payne::{Event, Input, MoveDir, State, Vec3};

fn dodge_forward(t: &mut T) {
    t.hold(0.3, fwd());
    t.step(Input {
        shootdodge: true,
        ..fwd()
    });
}

#[test]
fn shootdodge_dives_slows_the_world_lands_prone_and_gets_up() {
    let mut t = flat();
    t.sim.tuning.shootdodge_slowmo = 1.0; // MP2's own behaviour

    let sd = t.sim.moves.shootdodge[0].clone();
    // Spend a few rounds: frame 0's C_Reload refills the magazine.
    t.select(0);
    for _ in 0..3 {
        t.tap(still(), |i| i.fire = true);
        t.hold(0.3, still());
    }
    assert_eq!(t.sim.player.weapon().mag, 13);

    dodge_forward(&mut t);
    let start = t.sim.player.pos;
    assert!(matches!(
        t.state(),
        State::Shootdodge {
            dir: MoveDir::Forward,
            slowmo: true,
            ..
        }
    ));
    assert!(
        t.events
            .contains(&Event::ShootdodgeStarted { slowmo: true })
    );
    t.step(fwd());
    assert_eq!(t.sim.player.weapon().mag, 16);

    // Frame 10: game speed to 0.2 over 0.3 s. Frame 12: airborne, bullet time blocked.
    let mut min_scale: f32 = 1.0;
    let mut real = 0.0;
    let mut was_airborne = false;
    while matches!(t.state(), State::Shootdodge { .. }) {
        t.step(still());
        real += DT;
        min_scale = min_scale.min(t.sim.world_time_scale());
        if let State::Shootdodge {
            airborne: true,
            landed: false,
            ..
        } = t.state()
        {
            was_airborne = true;
            assert!(t.sim.bullet_time.blocked);
            assert!(t.sim.player.pos.y > 0.0);
        }
        assert!(real < 10.0, "{:?}", t.trace);
    }
    assert!(was_airborne);
    assert!((min_scale - 0.2).abs() < 1e-4, "slowest world {min_scale}");
    // 20 frames of the dive at ~0.2 speed: far longer than the 1.7 s animation.
    assert!(real > 3.5 && real < 5.5, "dive took {real}s real");
    assert!(
        matches!(
            t.state(),
            State::Prone {
                dir: MoveDir::Forward,
                ..
            }
        ),
        "{:?}",
        t.trace
    );
    assert!(t.events.contains(&Event::ShootdodgeLanded));
    let dist = t.sim.player.pos.distance(start);
    assert!((dist - sd.distance).abs() < 0.2, "dove {dist} m");

    // Lying still: stays down; world back to normal after frame 30's speed message.
    t.hold(1.0, still());
    assert!(matches!(t.state(), State::Prone { .. }));
    assert_eq!(t.sim.world_time_scale(), 1.0);
    assert!(
        t.sim.bullet_time.blocked,
        "blocked until the get-up's frame 20"
    );

    t.step(fwd());
    assert!(matches!(t.state(), State::GetUp { .. }));
    let up = t.until(3.0, still(), |s| s.player.state == State::Ground) + DT;
    assert!(
        (up - t.sim.moves.getup[0].duration()).abs() <= 2.5 * DT,
        "get-up {up}"
    );
    assert!(!t.sim.bullet_time.blocked);
    assert!(t.events.contains(&Event::GotUp));
}

#[test]
fn shootdodge_without_meter_dives_in_real_time() {
    let mut t = flat();
    t.sim.bullet_time.meter = 0.0;
    t.sim.tuning.regeneration_time = 1e9; // no passive refill during the run-up
    dodge_forward(&mut t);
    assert!(matches!(t.state(), State::Shootdodge { slowmo: false, .. }));
    let real = t.until(5.0, still(), |s| {
        !matches!(s.player.state, State::Shootdodge { .. })
    });
    assert_eq!(t.sim.world_time_scale(), 1.0);
    let anim = t.sim.moves.shootdodge[0].duration();
    assert!((real - anim).abs() < 3.0 * DT, "real-time dive took {real}");
    assert!(matches!(t.state(), State::Prone { .. }));
}

#[test]
fn shootdodge_directions_follow_the_stick() {
    let mut t = flat();
    t.step(Input {
        shootdodge: true,
        ..mv(-1.0, 0.0)
    });
    assert!(matches!(
        t.state(),
        State::Shootdodge {
            dir: MoveDir::Left,
            ..
        }
    ));
    t.until(10.0, still(), |s| {
        matches!(s.player.state, State::Prone { .. })
    });
    let p = t.sim.player.pos;
    assert!(p.x < -5.0 && p.z.abs() < 1e-3, "{p:?}");
    // The body keeps facing the aim; the clip is the directional dive.
    assert_eq!(t.sim.player.body_yaw, 0.0);
}

#[test]
fn holding_the_stick_gets_up_straight_from_the_dive_after_the_abort_frame() {
    let mut t = flat();
    dodge_forward(&mut t);
    t.until(10.0, fwd(), |s| {
        matches!(s.player.state, State::GetUp { .. })
    });
    assert!(!t.saw("prone"));
    let sd = &t.sim.moves.shootdodge[0];
    let abort_t = t.sim.tuning.can_abort_shootdodge_frame / sd.fps;
    assert!(abort_t < sd.duration());
}

#[test]
fn firing_while_prone() {
    let mut t = flat();
    dodge_forward(&mut t);
    t.until(10.0, still(), |s| {
        matches!(s.player.state, State::Prone { .. })
    });
    t.events.clear();
    t.tap(still(), |i| i.fire = true);
    let shot = t.events.iter().find_map(|e| match e {
        Event::Shot { origin, .. } => Some(*origin),
        _ => None,
    });
    let origin = shot.expect("fired while prone");
    assert!((origin.y - t.sim.player.pos.y - t.sim.tuning.muzzle_prone).abs() < 1e-4);
    assert!(matches!(t.state(), State::Prone { .. }));
    assert_eq!(t.sim.projectiles.len(), 1);
}

#[test]
fn turning_all_the_way_round_while_prone() {
    let mut t = flat();
    let tu = t.sim.tuning.clone();
    dodge_forward(&mut t);
    t.until(10.0, still(), |s| {
        matches!(s.player.state, State::Prone { .. })
    });
    t.hold(0.5, still()); // finish the slide
    let pos = t.sim.player.pos;
    let ticks = 120;
    let per_tick = std::f32::consts::TAU / ticks as f32;
    let mut body_turned = 0.0;
    let mut shots = 0;
    for k in 0..ticks {
        let before = t.sim.player.body_yaw;
        let fire = k % 30 == 0;
        t.step(Input {
            look: [per_tick, 0.0],
            fire,
            ..Default::default()
        });
        body_turned += payne::math::wrap_angle(t.sim.player.body_yaw - before);
        if t.events.iter().any(|e| matches!(e, Event::Shot { .. })) {
            shots += 1;
            t.events.clear();
        }
        let p = &t.sim.player;
        assert!(
            matches!(p.state, State::Prone { .. }),
            "got up while turning"
        );
        assert!(payne::math::wrap_angle(p.yaw - p.body_yaw).abs() <= tu.prone_twist + 1e-4);
    }
    t.hold(1.0, still());
    assert!(
        body_turned > std::f32::consts::TAU - tu.prone_twist - 0.05,
        "body turned {body_turned}"
    );
    assert!(
        payne::math::wrap_angle(t.sim.player.yaw).abs() < 1e-3,
        "aim back to the start"
    );
    assert_eq!(shots, 4);
    assert!(t.sim.player.pos.distance(pos) < 1e-5, "lying still");
}

#[test]
fn a_dodge_can_still_become_a_shootdodge_early() {
    let mut t = flat();
    t.hold(0.5, mv(1.0, 0.0));
    t.step(Input {
        crouch: true,
        ..mv(1.0, 0.0)
    });
    t.hold(5.0 / 30.0, mv(1.0, 0.0));
    t.step(Input {
        shootdodge: true,
        ..mv(1.0, 0.0)
    });
    assert!(matches!(
        t.state(),
        State::Shootdodge {
            dir: MoveDir::Right,
            ..
        }
    ));

    // Too late (after frame 13): stays a dodge.
    let mut t = flat();
    t.hold(0.5, mv(1.0, 0.0));
    t.step(Input {
        crouch: true,
        ..mv(1.0, 0.0)
    });
    t.hold(0.8, mv(1.0, 0.0)); // the dodge runs at game speed 0.7: frame ~17
    t.step(Input {
        shootdodge: true,
        ..mv(1.0, 0.0)
    });
    assert!(matches!(t.state(), State::Dodge { .. }));
}

#[test]
fn standing_shootdodge_in_bullet_time_spins_and_reloads() {
    let mut t = flat();
    t.select(5); // MP5
    for _ in 0..10 {
        t.step(Input {
            fire: true,
            ..Default::default()
        });
    }
    t.step(still());
    let mag = t.sim.player.weapon().mag;
    assert!(mag < 30);
    t.tap(still(), |i| i.bullet_time = true);
    t.step(Input {
        shootdodge: true,
        ..Default::default()
    });
    assert!(matches!(t.state(), State::Spin { .. }));
    t.step(still());
    assert!(
        t.sim.player.invulnerable(),
        "C_SetDodgeImmortal(true) at frame 0"
    );
    t.until(10.0, still(), |s| s.player.state == State::Ground);
    assert_eq!(t.sim.player.weapon().mag, 30, "C_Reload at frame 38");
    assert!(!t.sim.player.invulnerable());
    assert!(t.sim.player.pos.distance(Vec3::ZERO) < 1e-4);
}

#[test]
fn by_default_the_dive_slows_the_world() {
    let mut t = flat();
    assert_eq!(t.sim.tuning.shootdodge_slowmo, 1.0, "MP2's behaviour is the default");
    dodge_forward(&mut t);
    assert!(matches!(t.state(), State::Shootdodge { slowmo: true, .. }));
    let mut real = 0.0;
    while matches!(t.state(), State::Shootdodge { .. }) {
        t.step(fwd());
        real += DT;
    }
    assert!(real > 3.5, "the slow dive lasts {real}s real");
}

#[test]
fn with_dive_slowmo_off_bullet_time_carries_on_through_the_dive() {
    let mut t = flat();
    t.sim.tuning.shootdodge_slowmo = 0.0;
    t.tap(still(), |i| i.bullet_time = true);
    t.hold(0.5, still());
    assert!(t.sim.bullet_time.active);
    dodge_forward(&mut t);
    while matches!(t.state(), State::Shootdodge { .. }) {
        t.step(fwd());
        assert!(t.sim.bullet_time.active, "the dive must not switch bullet time off");
        assert!(!t.sim.bullet_time.blocked);
    }
}

#[test]
fn with_dive_slowmo_off_only_bullet_time_slows_the_world() {
    let mut t = flat();
    t.sim.tuning.shootdodge_slowmo = 0.0;
    dodge_forward(&mut t);
    assert!(matches!(t.state(), State::Shootdodge { slowmo: false, .. }));
    let mut min_speed: f32 = 1.0;
    while matches!(t.state(), State::Shootdodge { .. }) {
        t.step(fwd());
        min_speed = min_speed.min(t.sim.world_time_scale());
    }
    assert_eq!(min_speed, 1.0, "the dive must not slow the world");
}
