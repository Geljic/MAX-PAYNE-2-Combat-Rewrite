//! Enemy AI: perception, MP2-style aiming, enemy bullets, cover, friendly fire. Each test scripts a
//! situation and checks the end state.

mod common;

use std::f32::consts::PI;

use common::*;
use payne::math::yaw_dir;
use payne::{Aabb, Event, Mode, Sim, TargetId, TestWorld, Vec3, World};

/// Yaw that faces from `from` towards `to`.
fn yaw_to(from: Vec3, to: Vec3) -> f32 {
    payne::math::dir_yaw(to - from)
}

fn mode(t: &T, id: TargetId) -> Mode {
    t.sim.target(id).unwrap().ai.as_ref().unwrap().mode
}

fn ai_mut(t: &mut T, id: TargetId) -> &mut payne::Ai {
    t.sim
        .targets
        .iter_mut()
        .find(|x| x.id == id)
        .unwrap()
        .ai
        .as_mut()
        .unwrap()
}

/// Perfect aim: no inaccuracy, no weapon scatter, exact bullet speed, no sidesteps.
fn sharpshooter(t: &mut T, id: TargetId) {
    let ai = ai_mut(t, id);
    ai.skin.class.initial_inaccuracy = 0.0;
    ai.skin.class.stable_inaccuracy = 0.0;
    for w in &mut t.sim.weapons {
        w.spread_base = 0.0;
        w.enemy_projectile_speed_random = 0.0;
    }
    t.sim.enemies.tuning.strafe_time_max = 0.0;
}

fn enemy_shots(t: &T, id: TargetId) -> usize {
    t.count(|e| matches!(e, Event::EnemyShot { enemy, .. } if *enemy == id))
}

#[test]
fn notices_max_in_its_field_of_view_but_not_behind() {
    // In front: combat within half a second, one alert.
    let mut t = flat();
    let at = Vec3::new(0.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    t.hold(0.5, still());
    assert_eq!(mode(&t, e), Mode::Combat);
    assert_eq!(t.count(|ev| *ev == Event::EnemyAlerted { enemy: e }), 1);

    // Facing away from a silent Max: stays idle.
    let mut t = flat();
    let e = t.sim.spawn_enemy(at, 0.0, "C01_Mafioso_A");
    t.hold(3.0, still());
    assert_eq!(mode(&t, e), Mode::Idle);
    assert_eq!(t.count(|ev| matches!(ev, Event::EnemyAlerted { .. })), 0);

    // The field of vision is 150°: 60° off the facing is seen, 80° is not.
    for (off, seen) in [(60.0f32, true), (80.0, false)] {
        let mut t = flat();
        let e = t
            .sim
            .spawn_enemy(at, yaw_to(at, Vec3::ZERO) + off.to_radians(), "C01_Mafioso_A");
        t.hold(0.1, still());
        assert_eq!(mode(&t, e) == Mode::Combat, seen, "{off}° off");
    }
}

#[test]
fn hears_a_gunshot_behind_it_and_turns_round() {
    let mut t = flat();
    t.sim = Sim::new(Vec3::ZERO, PI); // Max faces +Z, away from the enemy
    let at = Vec3::new(0.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, 0.0, "C01_Mafioso_A"); // faces -Z, away from Max
    t.hold(1.0, still());
    assert_eq!(mode(&t, e), Mode::Idle);
    t.tap(still(), |i| i.fire = true);
    assert!(t.events.contains(&Event::EnemyAlerted { enemy: e }));
    t.hold(2.0, still());
    assert_eq!(mode(&t, e), Mode::Combat);
    let yaw = t.sim.target(e).unwrap().yaw;
    assert!(
        yaw_dir(yaw).dot(yaw_dir(yaw_to(at, Vec3::ZERO))) > 0.99,
        "faces Max: yaw {yaw}"
    );
}

#[test]
fn hears_running_footsteps_only_close_by() {
    // Group two (running) reaches 5 m: an enemy facing away 4 m off hears Max run past.
    let mut t = flat();
    let at = Vec3::new(4.0, 0.0, -2.0);
    let near = t.sim.spawn_enemy(at, -PI / 2.0, "C01_Mafioso_A"); // faces +X, away
    // Out of earshot, and more than alert_radius (15 m) from the near one, which spots Max.
    let far_at = Vec3::new(25.0, 0.0, -2.0);
    let far = t.sim.spawn_enemy(far_at, -PI / 2.0, "C01_Mafioso_A");
    // Neither may shoot: a gunshot (group one, 1000 m) would alert everyone.
    ai_mut(&mut t, near).cooldown = 100.0;
    ai_mut(&mut t, far).cooldown = 100.0;
    t.hold(1.0, fwd());
    assert!(t.events.contains(&Event::EnemyAlerted { enemy: near }));
    assert!(!t.events.contains(&Event::EnemyAlerted { enemy: far }));
}

#[test]
fn spotting_max_alerts_enemies_nearby() {
    let mut t = flat();
    let a_at = Vec3::new(0.0, 0.0, -10.0);
    let a = t.sim.spawn_enemy(a_at, yaw_to(a_at, Vec3::ZERO), "C01_Mafioso_A");
    ai_mut(&mut t, a).cooldown = 100.0; // A never fires, so B can only learn from A
    let b = t.sim.spawn_enemy(Vec3::new(5.0, 0.0, -14.0), 0.0, "C01_Mafioso_A");
    let c = t.sim.spawn_enemy(Vec3::new(0.0, 0.0, -40.0), 0.0, "C01_Mafioso_A");
    t.hold(0.2, still());
    assert!(t.events.contains(&Event::EnemyAlerted { enemy: a }));
    assert!(t.events.contains(&Event::EnemyAlerted { enemy: b }));
    assert!(!t.events.contains(&Event::EnemyAlerted { enemy: c }), "30 m off");
    assert_eq!(enemy_shots(&t, a), 0);
}

#[test]
fn aim_settles_from_initial_to_stable_inaccuracy() {
    // Nine C01 enemies 25 m away on an arc in front of Max shoot for 6 s. Each shot's miss
    // distance at 25 m, divided by the inaccuracy radius at that moment (C01 4.0 → 2.5 m per 25 m
    // over 3 s, × 1.3 on normal), is uniform over a disc: mean 2/3, never above 1.
    let mut t = flat();
    for w in &mut t.sim.weapons {
        w.spread_base = 0.0;
    }
    t.sim.enemies.tuning.strafe_time_max = 0.0;
    t.sim.player.health = 1e9;
    let mut ids = Vec::new();
    for k in 0..9 {
        let a = (-60.0 + 15.0 * k as f32).to_radians();
        let at = yaw_dir(a) * 25.0;
        ids.push(t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A"));
    }
    let (mut early, mut late, mut ratios) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..(6.0 / DT) as usize {
        let n0 = t.events.len();
        t.step(still());
        let chest = t.sim.player.chest(&t.sim.tuning);
        for ev in &t.events[n0..] {
            let Event::EnemyShot { enemy, origin, dir } = ev else { continue };
            let ai = t.sim.target(*enemy).unwrap().ai.as_ref().unwrap();
            let to = (chest - *origin).normalize_or_zero();
            let ang = dir.dot(to).clamp(-1.0, 1.0).acos();
            // The aim itself is on target (error well under a degree), so the miss is the scatter.
            let miss = 25.0 * (ang - ai.aim_error).max(0.0).tan();
            let r = ai.inaccuracy(&t.sim.difficulty);
            ratios.push(miss / r);
            if ai.aim_time < 0.5 {
                early.push(miss);
            } else if ai.aim_time > 3.0 {
                late.push(miss);
            }
        }
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    assert!(ratios.len() > 150, "{} shots", ratios.len());
    assert!(ratios.iter().all(|r| *r <= 1.02), "max {:?}", ratios.iter().cloned().fold(0.0, f32::max));
    let m = mean(&ratios);
    assert!((0.58..0.75).contains(&m), "mean miss / radius {m}");
    // 4.0 × 1.3 = 5.2 m early, 2.5 × 1.3 = 3.25 m settled.
    let (e, l) = (mean(&early), mean(&late));
    assert!((2.6..4.3).contains(&e), "early mean miss {e} ({} shots)", early.len());
    assert!((1.7..2.6).contains(&l), "late mean miss {l} ({} shots)", late.len());
}

#[test]
fn holds_fire_until_the_aim_is_within_trigger_happiness() {
    let mut t = flat();
    t.sim = Sim::new(Vec3::ZERO, PI);
    t.sim.player.health = 1e9;
    let at = Vec3::new(0.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, 0.0, "C01_Mafioso_A");
    t.sim.enemies.tuning.strafe_time_max = 0.0;
    t.tap(still(), |i| i.fire = true); // the gunshot makes it turn round
    let gate = 35f32.to_radians();
    let (mut seen_but_held, mut shots) = (0, 0);
    for _ in 0..120 {
        let n0 = t.events.len();
        t.step(still());
        let ai = t.sim.target(e).unwrap().ai.as_ref().unwrap();
        let fired = t.events[n0..]
            .iter()
            .any(|ev| matches!(ev, Event::EnemyShot { .. }));
        if fired {
            shots += 1;
            assert!(ai.aim_error <= gate + 1e-3, "fired {}° off", ai.aim_error.to_degrees());
        } else if ai.sees && ai.aim_error > gate && shots == 0 {
            seen_but_held += 1;
        }
    }
    assert!(seen_but_held > 0, "saw Max before the aim came round");
    assert!(shots > 0);
}

#[test]
fn enemy_bullets_fly_at_50_and_hit_max_for_the_weapon_damage() {
    let mut t = flat();
    let at = Vec3::new(0.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    sharpshooter(&mut t, e);
    t.until(1.0, still(), |s| s.projectiles.iter().any(|p| p.owner == Some(e)));
    let shot_tick = t.ticks;
    let p0 = *t.sim.projectiles.iter().find(|p| p.owner == Some(e)).unwrap();
    assert_eq!(p0.speed, 50.0);
    let player_shots = t.sim.projectiles.iter().filter(|p| p.owner.is_none()).count();
    assert_eq!(player_shots, 0);
    t.step(still());
    let p1 = t.sim.projectiles.iter().find(|p| p.owner == Some(e)).unwrap();
    assert!(((p1.pos - p0.pos).length() - 50.0 * DT).abs() < 1e-3);
    t.until(1.0, still(), |s| s.player.health < 250.0);
    // Muzzle to the front of the 0.35 m hit capsule, at 50 m/s.
    let dist = p0.pos.horizontal().length() - t.sim.tuning.player_hit_radius;
    let secs = (t.ticks - shot_tick) as f32 * DT;
    assert!((secs - dist / 50.0).abs() <= 2.0 * DT, "{secs} s for {dist} m");
    assert_eq!(t.sim.player.health, 250.0 - 17.0);
    let hit = t
        .events
        .iter()
        .find_map(|ev| match ev {
            Event::PlayerHit { damage, dir } => Some((*damage, *dir)),
            _ => None,
        })
        .unwrap();
    assert_eq!(hit.0, 17.0);
    assert!(hit.1.z > 0.99, "travelling +Z: {:?}", hit.1);
}

#[test]
fn bullet_time_slows_enemy_bullets() {
    let mut t = flat();
    t.sim.player.health = 1e9;
    t.tap(still(), |i| i.bullet_time = true);
    t.hold(0.5, still()); // fully faded in
    let scale = t.sim.world_time_scale();
    assert!((scale - 0.6).abs() < 1e-3, "{scale}");
    let at = Vec3::new(0.0, 0.0, -30.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    sharpshooter(&mut t, e);
    t.until(2.0, still(), |s| s.projectiles.iter().any(|p| p.owner == Some(e)));
    let p0 = t.sim.projectiles.iter().find(|p| p.owner == Some(e)).unwrap().pos;
    t.hold(10.0 * DT, still());
    let p1 = t
        .sim
        .projectiles
        .iter()
        .filter(|p| p.owner == Some(e))
        .map(|p| p.pos)
        .min_by(|a, b| a.length().total_cmp(&b.length()))
        .unwrap();
    let moved = (p1 - p0).length();
    assert!(
        (moved - 50.0 * 0.6 * 10.0 * DT).abs() < 0.05,
        "moved {moved} m in 10 ticks"
    );
}

#[test]
fn walls_stop_enemy_sight_and_bullets() {
    let w = TestWorld::flat().with_box(Aabb::block(0.0, -5.0, 6.0, 3.0, 0.5));
    let mut t = on(w, Vec3::ZERO);
    let at = Vec3::new(0.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    t.hold(3.0, still());
    assert_eq!(mode(&t, e), Mode::Idle);
    assert_eq!(enemy_shots(&t, e), 0);
    // A bullet fired through the wall anyway stops on it.
    t.sim.projectiles.push(payne::Projectile {
        pos: Vec3::new(0.0, 1.4, -9.0),
        prev: Vec3::new(0.0, 1.4, -9.0),
        dir: Vec3::Z,
        speed: 50.0,
        damage: 17.0,
        impulse: 1.0,
        weapon: 0,
        travelled: 0.0,
        range: 100.0,
        age: 0.0,
        owner: Some(e),
    });
    t.hold(0.5, still());
    assert_eq!(t.sim.player.health, 250.0);
    assert!(t.events.iter().any(|ev| matches!(ev, Event::Hit { target: None, .. })));
    assert!(!t.events.iter().any(|ev| matches!(ev, Event::PlayerHit { .. })));
}

#[test]
fn enemy_fire_kills_max_and_then_stops() {
    let mut t = flat();
    let at = Vec3::new(0.0, 0.0, -8.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    sharpshooter(&mut t, e);
    t.sim.player.health = 20.0;
    t.until(3.0, still(), |s| !s.player.alive());
    assert!(t.events.contains(&Event::PlayerDied));
    assert_eq!(t.count(|ev| matches!(ev, Event::PlayerHit { .. })), 2);
    assert!(t.sim.corpses.iter().any(|c| c.owner.is_none()));
    let shots = enemy_shots(&t, e);
    t.hold(2.0, still());
    assert_eq!(enemy_shots(&t, e), shots, "no shooting at a corpse");
}

#[test]
fn holds_fire_while_an_ally_stands_in_the_way() {
    let mut t = flat();
    t.sim.player.health = 1e9;
    let a_at = Vec3::new(0.0, 0.0, -20.0);
    let a = t.sim.spawn_enemy(a_at, yaw_to(a_at, Vec3::ZERO), "C01_Mafioso_A");
    sharpshooter(&mut t, a);
    // B stands in A's line of fire, facing away from Max (so B never sees him).
    let b = t.sim.spawn_enemy(Vec3::new(0.0, 0.0, -10.0), 0.0, "C01_Mafioso_A");
    t.sim.enemies.tuning.strafe_time_max = 0.0;
    t.hold(2.0, still());
    assert_eq!(enemy_shots(&t, a), 0, "held fire");
    assert_eq!(t.sim.target(b).unwrap().health, 50.0);
    // B steps aside: A opens fire.
    if let Some(bt) = t.sim.targets.iter_mut().find(|x| x.id == b) {
        bt.pos.x = 3.0;
    }
    t.hold(1.0, still());
    assert!(enemy_shots(&t, a) > 0);
}

#[test]
fn enemy_bullets_hit_other_enemies() {
    let mut t = flat();
    t.sim.player.health = 1e9;
    let a_at = Vec3::new(0.0, 0.0, -20.0);
    let a = t.sim.spawn_enemy(a_at, yaw_to(a_at, Vec3::ZERO), "C01_Mafioso_A");
    // B stands in front of A's gun, facing away from Max; a stray round from A (fired as if
    // B had stepped in at the last moment) hits B and never A itself.
    let b = t.sim.spawn_enemy(Vec3::new(0.0, 0.0, -10.0), 0.0, "C01_Mafioso_A");
    let meter = t.sim.bullet_time.meter;
    t.sim.projectiles.push(payne::Projectile {
        pos: Vec3::new(0.0, 1.3, -19.5),
        prev: Vec3::new(0.0, 1.3, -19.5),
        dir: Vec3::Z,
        speed: 50.0,
        damage: 17.0,
        impulse: 1.0,
        weapon: 0,
        travelled: 0.0,
        range: 100.0,
        age: 0.0,
        owner: Some(a),
    });
    t.until(2.0, still(), |s| s.target(b).is_none_or(|x| x.health < 50.0));
    assert!(t.events.iter().any(|ev| matches!(
        ev,
        Event::Hit { target: Some(id), .. } if *id == b
    )));
    assert_eq!(t.sim.target(a).unwrap().health, 50.0, "never hits itself");
    t.hold(3.0, still());
    if !t.sim.target(b).unwrap().alive() {
        assert!(t.sim.bullet_time.meter <= meter + 1e-4, "no bonus for a friendly kill");
    }
}

#[test]
fn takes_cover_behind_a_crate_when_shot_at_then_peeks() {
    // A 1.2 m crate stands to the right of the enemy's line to Max.
    let crate_box = Aabb::block(2.5, -10.0, 1.6, 1.2, 0.6);
    let mut t = on(TestWorld::flat().with_box(crate_box), Vec3::ZERO);
    t.accurate();
    t.sim.player.health = 1e9;
    t.sim.enemies.tuning.strafe_time_max = 0.0;
    let at = Vec3::new(0.0, 0.0, -12.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    t.hold(0.3, still());
    assert_eq!(mode(&t, e), Mode::Combat);
    // A near miss a metre to its side.
    let aim = t.aim_at(still(), Vec3::new(1.0, 1.2, -12.0));
    t.step(aim);
    t.tap(aim, |i| i.fire = true);
    t.until(4.0, aim, |s| {
        matches!(s.target(e).unwrap().ai.as_ref().unwrap().mode, Mode::InCover { .. })
    });
    t.hold(0.1, aim);
    let x = t.sim.target(e).unwrap();
    let chest = t.sim.player.chest(&t.sim.tuning);
    let low = x.pos + Vec3::Y * 0.9;
    let d = low - chest;
    let h = t.w.raycast(chest, d.normalize_or_zero(), d.length());
    assert!(h.is_some(), "hidden behind the crate at {:?}", x.pos);
    assert!(x.pos.distance(Vec3::new(2.5, 0.0, -10.0)) < 1.6, "next to the crate: {:?}", x.pos);
    let anim = x.enemy_anim().unwrap();
    assert_eq!(anim.slot, "CROUCH");
    // Later it stands up to shoot over the crate.
    t.until(4.0, aim, |s| {
        matches!(
            s.target(e).unwrap().ai.as_ref().unwrap().mode,
            Mode::InCover { peeking: true, .. }
        )
    });
    let shots = enemy_shots(&t, e);
    t.hold(1.0, aim);
    assert!(enemy_shots(&t, e) > shots, "shoots while peeking");
}

#[test]
fn stays_crouched_behind_cover_that_hides_a_crouched_eye() {
    // Max crouches in line with a 1.2 m crate; the gunman behind it is hidden once he crouches. On
    // the tick he arrives he is still standing, so his first sight test must not count the cover
    // as flanked: he crouches (CROUCH, InCover) for the whole hiding time instead of standing up.
    let crate_box = Aabb::block(0.0, -8.0, 2.0, 1.2, 0.6);
    let mut t = on(TestWorld::flat().with_box(crate_box), Vec3::ZERO);
    t.sim.player.health = 1e9;
    let crouch = || {
        let mut i = still();
        i.crouch = true;
        i
    };
    t.hold(0.5, crouch());
    let at = Vec3::new(4.0, 0.0, -11.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    t.hold(0.3, crouch());
    ai_mut(&mut t, e).on_shot_at(Vec3::new(0.0, 1.0, 0.0));
    t.until(6.0, crouch(), |s| {
        matches!(s.target(e).unwrap().ai.as_ref().unwrap().mode, Mode::InCover { .. })
    });
    for _ in 0..60 {
        t.step(crouch());
        let x = t.sim.target(e).unwrap();
        let ai = x.ai.as_ref().unwrap();
        assert!(
            matches!(ai.mode, Mode::InCover { .. }),
            "left cover after arriving: {:?} at {:?}",
            ai.mode,
            x.pos
        );
        assert!(ai.crouched);
    }
}

#[test]
fn runs_to_cover_on_a_run_clip() {
    let crate_box = Aabb::block(2.5, -10.0, 1.6, 1.2, 0.6);
    let mut t = on(TestWorld::flat().with_box(crate_box), Vec3::ZERO);
    t.sim.player.health = 1e9;
    let at = Vec3::new(0.0, 0.0, -12.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    t.hold(0.3, still());
    ai_mut(&mut t, e).on_shot_at(Vec3::new(0.0, 1.6, 0.0));
    let mut slots = Vec::new();
    for _ in 0..60 {
        t.step(still());
        let a = t.sim.target(e).unwrap().enemy_anim().unwrap();
        if slots.last() != Some(&a.slot) {
            slots.push(a.slot);
        }
    }
    assert!(slots.iter().any(|s| s.starts_with("RUN")), "{slots:?}");
}

#[test]
fn stops_shooting_while_flinching() {
    let mut t = flat();
    t.accurate();
    t.sim.player.health = 1e9;
    let at = Vec3::new(0.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::ZERO), "C01_Mafioso_A");
    t.sim.enemies.tuning.strafe_time_max = 0.0;
    t.sim.enemies.tuning.cover_radius = 0.0; // stay put
    t.hold(0.5, still());
    let leg = t.sim.target(e).unwrap().to_world(Vec3::new(0.11, 0.5, 0.0));
    let aim = t.aim_at(still(), leg);
    t.step(aim);
    t.tap(aim, |i| i.fire = true);
    t.until(1.0, aim, |s| {
        matches!(s.target(e).unwrap().state, payne::TargetState::Flinch { .. })
    });
    assert_eq!(t.sim.target(e).unwrap().enemy_anim().unwrap().slot, "SMALLDAMAGE");
    loop {
        let shots = enemy_shots(&t, e);
        t.step(still());
        if !matches!(t.sim.target(e).unwrap().state, payne::TargetState::Flinch { .. }) {
            break;
        }
        assert_eq!(enemy_shots(&t, e), shots, "fired while flinching");
    }
}

#[test]
fn kill_bonus_uses_the_difficulty_multiplier() {
    let mut t = flat();
    t.accurate();
    t.sim.difficulty.bullet_time_gained_per_kill_multiplier = 2.0;
    t.sim.bullet_time.meter = 0.5;
    let at = Vec3::new(0.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, 0.0, "C01_Mafioso_A"); // facing away: no return fire
    let head = t.sim.target(e).unwrap().to_world(Vec3::new(0.0, 1.66, 0.0));
    let aim = t.aim_at(still(), head);
    t.step(aim);
    let m0 = t.sim.bullet_time.meter;
    t.tap(aim, |i| i.fire = true);
    t.until(1.0, aim, |s| !s.target(e).unwrap().alive());
    let gained = t.sim.bullet_time.meter - m0;
    // 0.12 × 2, plus a few ticks of the 90 s recharge.
    assert!((gained - 0.24).abs() < 0.01, "gained {gained}");
    assert!(t.sim.target(e).unwrap().enemy_anim().is_none(), "dead: ragdoll only");
    assert!(t.sim.corpse(e).is_some());
}

#[test]
fn enemy_hits_max_lying_after_a_dive_low_down() {
    // The prone hit capsule lies along the body: a bullet at knee height over him misses,
    // one at 0.2 m hits.
    let mut t = flat();
    t.sim.player.state = payne::State::Prone {
        t: 0.0,
        dir: payne::MoveDir::Forward,
    };
    let (a, b, r) = t.sim.player.hit_capsule(&t.sim.tuning);
    assert!(a.y < 0.4 && b.y < 0.4 && r <= 0.35);
    assert!((a - b).horizontal().length() > 0.8, "lies along the body");
    let fire = |t: &mut T, y: f32| {
        t.sim.projectiles.push(payne::Projectile {
            pos: Vec3::new(-5.0, y, 0.0),
            prev: Vec3::new(-5.0, y, 0.0),
            dir: Vec3::X,
            speed: 50.0,
            damage: 10.0,
            impulse: 1.0,
            weapon: 0,
            travelled: 0.0,
            range: 100.0,
            age: 0.0,
            owner: Some(TargetId(99)),
        });
        t.hold(0.3, still());
    };
    fire(&mut t, 1.0);
    assert_eq!(t.sim.player.health, 250.0);
    fire(&mut t, 0.2);
    assert_eq!(t.sim.player.health, 240.0);
}

#[test]
fn searches_the_last_known_position_then_goes_idle() {
    let w = TestWorld::flat().with_box(Aabb::block(0.0, -5.0, 6.0, 3.0, 0.5));
    let mut t = on(w, Vec3::new(5.0, 0.0, 0.0));
    let at = Vec3::new(5.0, 0.0, -10.0);
    let e = t.sim.spawn_enemy(at, yaw_to(at, Vec3::new(5.0, 0.0, 0.0)), "C01_Mafioso_A");
    t.sim.player.health = 1e9;
    t.sim.enemies.tuning.strafe_time_max = 0.0;
    t.sim.enemies.tuning.cover_radius = 0.0;
    t.hold(0.5, still());
    assert_eq!(mode(&t, e), Mode::Combat);
    // Max walks behind the wall.
    t.until(3.0, mv(-1.0, 0.0), |s| s.player.pos.x < 0.0);
    t.until(4.0, still(), |s| {
        matches!(s.target(e).unwrap().ai.as_ref().unwrap().mode, Mode::Search { .. })
    });
    // It goes there, sees him round the wall's end or ends up searching; either way, with Max
    // gone for good it returns to idle after idle_timeout (10 s).
    t.sim.player.pos = Vec3::new(0.0, 0.0, 200.0); // beyond sight range
    t.hold(1.0, still());
    t.until(25.0, still(), |s| s.target(e).unwrap().ai.as_ref().unwrap().mode == Mode::Idle);
    assert!(!t.sim.target(e).unwrap().ai.as_ref().unwrap().alerted);
}

#[test]
fn unknown_skins_and_no_data_use_c01_defaults() {
    let mut t = flat();
    let e = t.sim.spawn_enemy(Vec3::new(0.0, 0.0, -5.0), 0.0, "NoSuchSkin");
    let x = t.sim.target(e).unwrap();
    let ai = x.ai.as_ref().unwrap();
    assert_eq!(x.health, 50.0);
    assert_eq!(ai.skin.class.trigger_happiness_angle, 35.0);
    assert_eq!(ai.skin.style, payne::Style::Plain);
    assert_eq!(ai.speeds.run, t.sim.enemies.tuning.run_speed);
    let a = x.enemy_anim().unwrap();
    assert_eq!((a.slot, a.style), ("STAND", payne::Style::Plain));
    // Gallery targets have no brain.
    let g = t.sim.spawn_target(Vec3::new(3.0, 0.0, -5.0), 0.0);
    assert!(t.sim.target(g).unwrap().enemy_anim().is_none());
}

#[test]
fn real_data_enemies() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/generated/payne.data");
    if !std::path::Path::new(path).exists() {
        return;
    }
    let g = payne::GameData::load(path).unwrap();
    if g.enemies.skins.is_empty() {
        return;
    }
    assert_eq!(g.difficulty.name, "normal.default");
    assert_eq!(g.difficulty.initial_inaccuracy_multiplier, 1.3);
    let beretta = &g.weapons[0];
    assert_eq!(beretta.enemy_projectile_speed, 50.0);
    let mut sim = Sim::with_data(Vec3::ZERO, 0.0, g);
    let e = sim.spawn_enemy(Vec3::new(0.0, 0.0, -5.0), 0.0, "C01_Mafioso_A");
    let x = sim.target(e).unwrap();
    assert_eq!(x.health, 50.0);
    let ai = x.ai.as_ref().unwrap();
    assert_eq!(ai.skin.class.name, "C01");
    // Plain enemies run on the ENEMY/ set's jog, measured at about 2.4 m/s.
    assert!((ai.speeds.run - 2.41).abs() < 0.1, "{}", ai.speeds.run);
    let sharp = sim
        .enemies
        .skins
        .iter()
        .find(|s| s.style == payne::Style::Sharp)
        .map(|s| s.name.clone());
    if let Some(name) = sharp {
        let e = sim.spawn_enemy(Vec3::new(3.0, 0.0, -5.0), 0.0, &name);
        let ai = sim.target(e).unwrap().ai.as_ref().unwrap();
        assert!((ai.speeds.run - 4.22).abs() < 0.1, "{name}: {}", ai.speeds.run);
    }
}
