//! Combat mode for the range: a wave of real enemy skins placed around the test world's boxes
//! (so they have cover), a god switch for hands-free runs, and the reset that gets Max back up.
//!
//! Keys: F2 spawns the wave (replacing the paper targets; T in the range example puts them back),
//! Backspace / G resets Max (health, painkillers, weapons) and respawns the wave while combat
//! mode is on. The `--combat` flag of the range example and the `combat` script command do the same
//! as F2.

use bevy::prelude::*;

use super::PayneSim;
use crate::bullet_time::BulletTime;
use crate::math::Vec3 as PVec;
use crate::player::Player;
use crate::sim::Sim;

/// Combat mode state.
#[derive(Resource, Default)]
pub struct PayneCombat {
    /// A wave has been spawned (Backspace / G respawns it).
    pub active: bool,
    /// Max takes no damage (the hit feedback still plays); for hands-free screenshot runs.
    pub god: bool,
    /// Where Max stands after a reset.
    pub spawn: PVec,
    pub spawn_yaw: f32,
}

/// The wave: skin, feet position (x, z) and the way each gunman faces at his post (rad, 0 = away
/// from the range start; they start looking elsewhere so they are idle until they hear or see
/// Max). Spread 15-30 m from the start, next to the test world's boxes (the low wall at z -12, the
/// crate at (5, -6), the block at (-7, -8), the far wall at z -30) so cover points are near.
/// ESTIMATE: placement by eye; MP2 levels place enemies by hand too.
pub const WAVE: [(&str, f32, f32, f32); 7] = [
    ("C01_Mafioso_A", -3.0, -17.0, 0.9),
    ("C01_Mafioso_A", 3.5, -19.0, -0.9),
    ("C01_Mafioso_A", -11.0, -15.0, 0.9),
    ("C03_SemiCleaner_A", 8.0, -21.0, -0.9),
    ("C03_SemiCleaner_A", -6.0, -23.0, 0.9),
    ("C04_BlackOps_A", -2.0, -26.0, -0.9),
    ("C04_BlackOps_A", 7.0, -25.0, 0.9),
];

/// Replace the targets with the wave.
pub fn spawn_wave(sim: &mut Sim) {
    sim.targets.clear();
    sim.corpses.clear();
    sim.projectiles.clear();
    for (skin, x, z, yaw) in WAVE {
        sim.spawn_enemy(PVec::new(x, 0.0, z), yaw, skin);
    }
}

/// Stand Max up again: full health, painkillers and weapons, no bullets in flight, no corpses.
pub fn reset_player(sim: &mut Sim, spawn: PVec, yaw: f32) {
    sim.player = Player::new(spawn, yaw, &sim.weapons, &sim.tuning);
    sim.bullet_time = BulletTime::default();
    sim.projectiles.clear();
    sim.corpses.clear();
}

/// F2: spawn the wave. Backspace / G: reset Max (and the wave, in combat mode).
pub fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<PayneSim>,
    mut c: ResMut<PayneCombat>,
) {
    if keys.just_pressed(KeyCode::F2) {
        spawn_wave(&mut sim.0);
        c.active = true;
    }
    if keys.just_pressed(KeyCode::Backspace) || keys.just_pressed(KeyCode::KeyG) {
        let was_dead = !sim.0.player.alive();
        if c.active {
            reset_player(&mut sim.0, c.spawn, c.spawn_yaw);
            spawn_wave(&mut sim.0);
        } else if was_dead {
            reset_player(&mut sim.0, c.spawn, c.spawn_yaw);
        }
    }
}
