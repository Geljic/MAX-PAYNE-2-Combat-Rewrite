//! Weapons, projectiles and hit reactions.
mod common;
use common::*;
use payne::{
    Aabb, Event, Hand, HitSide, HitZone, Input, ReactionKind, TargetState, TestWorld, Vec3,
};

fn shots(t: &T) -> usize {
    t.count(|e| matches!(e, Event::Shot { .. }))
}

#[test]
fn reload_timing_and_ammo_counts() {
    let mut t = flat();
    t.select(0); // Beretta: 16 + 80
    let def = t.sim.weapons[0].clone();
    assert_eq!(
        (t.sim.player.weapon().mag, t.sim.player.weapon().reserve),
        (16, 80)
    );
    for _ in 0..5 {
        t.tap(still(), |i| i.fire = true);
        t.hold(0.3, still());
    }
    assert_eq!(t.sim.player.weapon().mag, 11);
    t.tap(still(), |i| i.reload = true);
    let secs = t.until(5.0, still(), |s| s.player.weapon().reload.is_none());
    assert!(
        (secs + DT - def.reload_time).abs() <= 1.5 * DT,
        "reload took {secs}"
    );
    assert_eq!(
        (t.sim.player.weapon().mag, t.sim.player.weapon().reserve),
        (16, 75)
    );
    assert_eq!(t.count(|e| matches!(e, Event::ReloadDone { weapon: 0 })), 1);
}

#[test]
fn switching_weapons_cancels_a_reload() {
    let mut t = flat();
    t.select(0);
    t.tap(still(), |i| i.fire = true);
    t.tap(still(), |i| i.reload = true);
    t.hold(0.3, still());
    t.select(2);
    t.select(0);
    assert_eq!(t.sim.player.weapon().mag, 15);
    assert!(t.sim.player.weapon().reload.is_none());
    assert!(t.events.contains(&Event::ReloadCancelled { weapon: 0 }));
}

#[test]
fn empty_magazine_reloads_itself() {
    let mut t = flat();
    t.select(2); // Desert Eagle, 10 rounds, 3 shots/s
    for _ in 0..10 {
        t.tap(still(), |i| i.fire = true);
        t.hold(0.4, still());
    }
    assert_eq!(t.sim.player.weapon().mag, 0);
    assert!(t.sim.player.weapon().reload.is_some());
    t.hold(2.0, still());
    assert_eq!(t.sim.player.weapon().mag, 10);
}

#[test]
fn fire_rate_is_limited() {
    // MP5: 8 shots/s while held.
    let mut t = flat();
    t.select(5);
    t.events.clear();
    t.hold(
        1.5,
        Input {
            fire: true,
            ..Default::default()
        },
    );
    let n = shots(&t);
    assert!((12..=13).contains(&n), "MP5 fired {n} in 1.5 s");

    // Beretta is semi-automatic: holding fires once; clicking every other tick is capped at 4/s.
    let mut t = flat();
    t.select(0);
    t.events.clear();
    t.hold(
        1.0,
        Input {
            fire: true,
            ..Default::default()
        },
    );
    assert_eq!(shots(&t), 1);
    t.hold(0.5, still());
    t.events.clear();
    for _ in 0..30 {
        t.tap(still(), |i| i.fire = true);
    }
    let n = shots(&t);
    assert!((4..=5).contains(&n), "clicked out {n} in 1 s");
}

#[test]
fn spread_grows_with_sustained_fire_and_recovers() {
    let mut t = flat();
    t.select(5);
    let def = t.sim.weapons[5].clone();
    t.hold(
        1.0,
        Input {
            fire: true,
            ..Default::default()
        },
    );
    let grown = t.sim.player.weapon().spread(&def);
    assert!(grown > def.spread_base + 0.03, "spread {grown}");
    t.hold(1.0, still());
    assert_eq!(t.sim.player.weapon().spread(&def), def.spread_base);
}

#[test]
fn shotgun_fires_nine_pellets_and_dual_guns_alternate() {
    let mut t = flat();
    t.select(3);
    t.tap(still(), |i| i.fire = true);
    assert_eq!(t.sim.projectiles.len(), 9);

    let mut t = flat();
    t.select(1);
    let hands: Vec<Hand> = (0..4)
        .map(|_| {
            t.events.clear();
            t.tap(still(), |i| i.fire = true);
            t.hold(0.3, still());
            t.events
                .iter()
                .find_map(|e| {
                    if let Event::Shot { hand, .. } = e {
                        Some(*hand)
                    } else {
                        None
                    }
                })
                .unwrap()
        })
        .collect();
    assert_eq!(
        hands,
        vec![Hand::Right, Hand::Left, Hand::Right, Hand::Left]
    );
}

/// Ticks from firing until a Hit event at a wall 30 m away.
fn flight_ticks(t: &mut T) -> usize {
    t.events.clear();
    t.step(Input {
        fire: true,
        ..Default::default()
    });
    let mut n = 1;
    while t.count(|e| matches!(e, Event::Hit { .. })) == 0 {
        t.step(still());
        n += 1;
        assert!(n < 1000);
    }
    n
}

#[test]
fn projectiles_take_time_and_arrive_later_in_bullet_time() {
    let w = TestWorld::flat().with_box(Aabb::block(0.0, -31.0, 10.0, 4.0, 2.0));
    let mut t = on(w.clone(), Vec3::ZERO);
    t.accurate();
    t.select(0);
    let normal = flight_ticks(&mut t);
    // 30 m at 150 m/s = 0.2 s = 12 ticks.
    assert!((11..=13).contains(&normal), "{normal} ticks");

    let mut t = on(w, Vec3::ZERO);
    t.accurate();
    t.select(0);
    t.tap(still(), |i| i.bullet_time = true);
    t.hold(0.5, still()); // fully blended: world at 0.6
    let slow = flight_ticks(&mut t);
    assert!((19..=22).contains(&slow), "{slow} ticks in bullet time");
}

#[test]
fn headshot_does_more_damage_than_a_leg_shot() {
    let mut t = flat();
    t.accurate();
    t.select(0);
    let head = t
        .sim
        .spawn_target(Vec3::new(-1.0, 0.0, -8.0), std::f32::consts::PI);
    let leg = t
        .sim
        .spawn_target(Vec3::new(1.0, 0.0, -8.0), std::f32::consts::PI);
    let damage = |t: &T, id| {
        t.events.iter().find_map(|e| match e {
            Event::Hit {
                target: Some(x),
                damage,
                zone,
                ..
            } if *x == id => Some((*damage, *zone)),
            _ => None,
        })
    };
    let i = t.aim_at(still(), Vec3::new(-1.0, 1.66, -8.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.hold(0.3, i);
    let i = t.aim_at(still(), Vec3::new(1.11, 0.4, -8.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.hold(0.3, i);
    let (hd, hz) = damage(&t, head).expect("head hit");
    let (ld, lz) = damage(&t, leg).expect("leg hit");
    assert_eq!((hz, lz), (Some(HitZone::Head), Some(HitZone::Leg)));
    assert_eq!((hd, ld), (17.0 * 3.0, 17.0 * 0.5));
    assert!(
        !t.sim.target(head).unwrap().alive(),
        "51 damage kills a 50 hp enemy"
    );
    assert_eq!(t.sim.target(leg).unwrap().health, 50.0 - 8.5);
}

#[test]
fn flinch_direction_matches_the_side_hit() {
    let mut t = flat();
    t.accurate();
    t.select(0);
    // Facing -X: the player (at +Z of it) shoots its left side.
    let side = t
        .sim
        .spawn_target(Vec3::new(0.0, 0.0, -6.0), std::f32::consts::FRAC_PI_2);
    let i = t.aim_at(still(), Vec3::new(0.0, 1.2, -6.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.hold(0.1, i);
    let target = t.sim.target(side).unwrap();
    let TargetState::Flinch { reaction, .. } = target.state else {
        panic!("{:?}", target.state)
    };
    // From the side the hanging arm covers the torso.
    assert_eq!(
        (reaction.kind, reaction.from, reaction.zone),
        (ReactionKind::Flinch, HitSide::Left, HitZone::Arm)
    );
    let right = payne::math::yaw_right(target.yaw);
    assert!(reaction.push.dot(right) > 0.99, "shoved to its right");

    // Facing the player: from the front, shoved backwards.
    let mut t = flat();
    t.accurate();
    t.select(0);
    let front = t
        .sim
        .spawn_target(Vec3::new(0.0, 0.0, -6.0), std::f32::consts::PI);
    let i = t.aim_at(still(), Vec3::new(0.0, 1.2, -6.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.hold(0.1, i);
    let TargetState::Flinch { reaction, .. } = t.sim.target(front).unwrap().state else {
        panic!()
    };
    assert_eq!(reaction.from, HitSide::Front);
    t.hold(1.0, i);
    assert_eq!(t.sim.target(front).unwrap().state, TargetState::Idle);
}

#[test]
fn a_big_hit_staggers() {
    let mut t = flat();
    t.accurate();
    t.select(3); // pump shotgun: 9 pellets × 9
    let mut def = t.sim.target_def.clone();
    def.health = 500.0;
    let id = t
        .sim
        .spawn_target_with(Vec3::new(0.0, 0.0, -3.0), std::f32::consts::PI, def);
    let i = t.aim_at(still(), Vec3::new(0.0, 1.2, -3.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.hold(0.1, i);
    let tg = t.sim.target(id).unwrap();
    assert!(
        matches!(tg.state, TargetState::Stagger { .. }),
        "{:?}",
        tg.state
    );
    assert_eq!(tg.health, 500.0 - 81.0);
    let z0 = tg.pos.z;
    t.hold(1.0, i);
    assert!(t.sim.target(id).unwrap().pos.z < z0 - 0.2, "shoved back");
}

#[test]
fn aim_point_is_kept_away_from_the_character() {
    let w = TestWorld::flat().with_box(Aabb::block(0.0, -1.2, 4.0, 3.0, 0.4));
    let mut t = on(w, Vec3::ZERO);
    let chest = t.sim.player.chest(&t.sim.tuning);
    // Camera behind the player looking forward: the wall is 1 m in front of the chest.
    let ray = (chest + Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, -1.0));
    t.step(Input {
        camera_ray: Some(ray),
        ..Default::default()
    });
    let d = t.sim.player.aim_target.distance(chest);
    assert!((d - 1.5).abs() < 1e-3, "aim target {d} m away");
    // Far wall: aim exactly at the hit.
    let mut t = flat();
    let chest = t.sim.player.chest(&t.sim.tuning);
    t.step(Input {
        camera_ray: Some((chest, Vec3::new(0.0, -0.5, -1.0).normalize_or_zero())),
        ..Default::default()
    });
    assert!(
        t.sim.player.aim_target.y.abs() < 1e-4,
        "aims at the floor hit"
    );
}

#[test]
fn damage_does_not_stun_and_kills_the_player() {
    let mut t = flat();
    t.hold(0.5, fwd());
    let v = t.sim.player.vel.length();
    t.sim.damage_player(10.0, Vec3::new(1.0, 0.0, 0.0)); // from the left
    // MP2 does not stun Max: he keeps running, and the hit is only recorded for a visual flinch.
    assert_eq!(t.state(), payne::State::Ground);
    assert!(matches!(t.sim.player.last_hit, Some((_, HitSide::Left))));
    t.hold(0.1, fwd());
    assert!(t.sim.player.vel.length() >= v * 0.99, "kept running");
    t.sim.damage_player(1000.0, Vec3::new(1.0, 0.0, 0.0));
    assert!(!t.sim.player.alive());
    assert_eq!(t.sim.corpses.len(), 1);
    t.step(Input {
        fire: true,
        ..Default::default()
    });
    assert!(t.sim.projectiles.is_empty());
}
