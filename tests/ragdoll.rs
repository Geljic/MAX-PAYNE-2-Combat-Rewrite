//! Bodies: a kill hands the target to a ragdoll that falls, settles and stays in one piece.
mod common;
use common::*;
use payne::{Aabb, Event, Input, Ragdoll, SkeletonDef, TestWorld, Tuning, Vec3, World};

fn assert_sane(r: &Ragdoll, floor: f32) {
    for p in &r.particles {
        assert!(p.pos.is_finite() && p.vel.is_finite());
        assert!(
            p.pos.y >= floor + p.radius - 0.02,
            "particle below the floor: {:?}",
            p.pos
        );
    }
    assert!(
        r.bone_stretch() < 0.05,
        "bones stretched {:.1}%",
        r.bone_stretch() * 100.0
    );
}

#[test]
fn a_killed_target_ragdolls_and_settles_on_the_ground() {
    let mut t = flat();
    t.accurate();
    t.select(2);
    let id = t
        .sim
        .spawn_target(Vec3::new(0.0, 0.0, -6.0), std::f32::consts::PI);
    let i = t.aim_at(still(), Vec3::new(0.0, 1.66, -6.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.until(1.0, i, |s| s.corpse(id).is_some());
    assert!(t.events.iter().any(|e| matches!(e, Event::Kill { .. })));
    let secs = t.until(8.0, still(), |s| s.corpse(id).unwrap().ragdoll.is_asleep());
    let r = &t.sim.corpse(id).unwrap().ragdoll;
    assert_sane(r, 0.0);
    let top = r.particles.iter().map(|p| p.pos.y).fold(f32::MIN, f32::max);
    assert!(top < 0.7, "lying down (highest joint {top})");
    // Headshot travelling -Z: the head went that way.
    let head = r.particles[3].pos;
    assert!(head.z < -6.5, "head at {head:?}");
    println!("settled in {secs:.2} s");
}

#[test]
fn a_huge_impulse_does_not_explode() {
    let sk = SkeletonDef::humanoid();
    let tu = Tuning::default();
    let w = TestWorld::flat();
    let mut r = Ragdoll::from_rest(&sk, Vec3::ZERO, 0.3, Vec3::ZERO);
    r.apply_impulse(Vec3::new(0.0, 1.6, 0.0), Vec3::new(300.0, 150.0, -200.0));
    for _ in 0..(10.0 / DT) as usize {
        r.step(&w, DT, &tu);
        for p in &r.particles {
            assert!(p.pos.is_finite());
        }
    }
    assert!(r.is_asleep());
    assert_sane(&r, 0.0);
}

#[test]
fn bodies_rest_on_boxes_and_run_on_the_world_clock() {
    let sk = SkeletonDef::humanoid();
    let tu = Tuning::default();
    let w = TestWorld::flat().with_box(Aabb::new(
        Vec3::new(-5.0, 0.0, -5.0),
        Vec3::new(5.0, 1.0, 5.0),
    ));
    let mut r = Ragdoll::from_rest(&sk, Vec3::new(0.0, 1.0, 0.0), 0.0, Vec3::ZERO);
    r.apply_impulse(Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 0.0, 15.0));
    // Slow world: tiny steps must still settle (in more ticks).
    for _ in 0..(40.0 / DT) as usize {
        r.step(&w, DT * 0.25, &tu);
    }
    assert!(r.is_asleep());
    assert_sane(&r, 1.0);
    assert!(
        r.particles.iter().all(|p| p.pos.y < 1.75),
        "fell over onto the box top"
    );
    let _ = w.ground(0.0, 0.0, 0.0, 2.0);
}

#[test]
fn shooting_a_body_pushes_it() {
    let mut t = flat();
    t.accurate();
    t.select(2);
    let id = t
        .sim
        .spawn_target(Vec3::new(0.0, 0.0, -5.0), std::f32::consts::PI);
    let i = t.aim_at(still(), Vec3::new(0.0, 1.66, -5.0));
    t.step(i);
    t.step(Input { fire: true, ..i });
    t.until(8.0, still(), |s| {
        s.corpse(id).is_some_and(|c| c.ragdoll.is_asleep())
    });
    let before = t.sim.corpse(id).unwrap().ragdoll.particles[0].pos;
    let i = t.aim_at(still(), before);
    t.step(i);
    t.hold(0.5, i);
    t.step(Input { fire: true, ..i });
    t.hold(0.3, i);
    assert!(
        t.events
            .iter()
            .any(|e| matches!(e, Event::BodyHit { target: Some(x), .. } if *x == id))
    );
    assert!(
        !t.sim.corpse(id).unwrap().ragdoll.is_asleep()
            || t.sim.corpse(id).unwrap().ragdoll.particles[0].pos != before
    );
}
