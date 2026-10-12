//! Player health: painkillers and auto-heal from difficulty.txt (normal.default), death.

mod common;

use common::*;
use payne::{Event, Vec3};

const FROM_FRONT: Vec3 = Vec3::new(0.0, 0.0, 1.0);

#[test]
fn painkiller_heals_at_once_then_regenerates_to_its_total() {
    let mut t = flat();
    assert_eq!(t.sim.player.painkillers, 3);
    t.sim.damage_player(100.0, FROM_FRONT);
    t.hold(0.5, still());
    assert_eq!(t.sim.player.health, 150.0);
    t.tap(still(), |i| i.painkiller = true);
    // 5% of 250 at once.
    assert!(t.events.contains(&Event::Painkiller { health: 162.5 }));
    assert_eq!(t.sim.player.painkillers, 2);
    // Then 4% of 250 = 10 per second (the tap's second tick already counts).
    t.hold(2.0 - DT, still());
    let h = t.sim.player.health;
    assert!((h - 182.5).abs() < 0.3, "after 2 s: {h}");
    // Until the painkiller has given 20% of 250 = 50 in all.
    t.hold(4.0, still());
    assert!((t.sim.player.health - 200.0).abs() < 0.01, "{}", t.sim.player.health);
    assert!(t.sim.player.heal_pending.abs() < 1e-4);
}

#[test]
fn painkiller_is_refused_at_full_health_or_with_none_left() {
    let mut t = flat();
    t.tap(still(), |i| i.painkiller = true);
    assert_eq!(t.sim.player.painkillers, 3);
    assert_eq!(t.count(|e| matches!(e, Event::Painkiller { .. })), 0);
    t.sim.player.painkillers = 0;
    t.sim.damage_player(50.0, FROM_FRONT);
    t.tap(still(), |i| i.painkiller = true);
    assert_eq!(t.count(|e| matches!(e, Event::Painkiller { .. })), 0);
    assert_eq!(t.sim.player.health, 200.0);
}

#[test]
fn low_health_creeps_back_to_the_auto_heal_level() {
    let mut t = flat();
    t.sim.damage_player(240.0, FROM_FRONT); // 10 left
    t.hold(5.0, still());
    // 1% of 250 per second up to 15% (37.5).
    let h = t.sim.player.health;
    assert!((h - 22.5).abs() < 0.1, "after 5 s: {h}");
    t.hold(20.0, still());
    assert_eq!(t.sim.player.health, 37.5);
}

#[test]
fn dying_ends_the_painkiller_and_leaves_a_corpse() {
    let mut t = flat();
    t.sim.damage_player(200.0, FROM_FRONT);
    t.hold(0.5, still());
    t.tap(still(), |i| i.painkiller = true);
    t.sim.damage_player(500.0, FROM_FRONT);
    t.step(still());
    assert!(t.events.contains(&Event::PlayerDied));
    assert!(t.events.contains(&Event::PlayerHit {
        damage: 500.0,
        dir: FROM_FRONT
    }));
    t.hold(2.0, still());
    assert_eq!(t.sim.player.health, 0.0);
    assert_eq!(t.sim.corpses.iter().filter(|c| c.owner.is_none()).count(), 1);
}
