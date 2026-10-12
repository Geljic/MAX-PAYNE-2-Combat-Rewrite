//! Scripted input for hands-free visual checks: a timed list of what the player holds, plus
//! screenshots and quit commands. The script runs on the fixed clock, so it replays exactly.
//!
//! One line per step, `<seconds> <tokens...>`; `#` starts a comment. A line that names inputs
//! replaces the held set from that time on (a bare time releases everything).
//!
//! ```text
//! 0.0 fwd                    # run forward
//! 0.4 fwd shootdodge         # dive
//! 1.0 fire aim=t0            # hold fire, steer the crosshair onto target 0's torso
//! 2.0 bt                     # bullet time held (a press toggles it)
//! 0.5 cam=front              # camera orbit: front | side | back | default
//! 0.5 cam=e3                 # camera 4 m in front of target 3 (an enemy), cam=player to come back
//! 1.2 shot=run_end           # screenshot <shots dir>/run_end.png
//! 3.0 quit
//! ```
//! Inputs: `fwd back left right fire mash reload jump crouch shootdodge bt`, `weapon=N` (one press),
//! `aim=t<N>` / `aim=h<N>` (target N's torso / head, in spawn order) or `aim=x,y,z`, `yaw=<rad/s>`,
//! `pitch=<rad/s>`. `inspect=N|last|off` (gallery camera on board N, the last board hit, or back to normal).
//! Combat testing: `combat` (spawn the enemy wave, replacing the targets; `aim=tN` then aims at enemy N),
//! `painkiller` (one press), `god=on|off` (Max takes no damage), `hurt=N` (N damage to Max from
//! the first enemy, or from straight ahead), `pos=x,z` (put Max there), `clear` (remove every target)
//! and `spawn=skin,x,z[,yaw]` (add one enemy, e.g. `spawn=C04_BlackOps_A,4,-16,0.9`).
//! `slowmo=on|off` sets shootdodge slow motion (the B key).
//! `cam=`, `shot=`, `quit`, `inspect=`, `weapon=`, `combat`, `clear`, `spawn=`, `painkiller`, `god=`, `slowmo=`,
//! `hurt=` and `pos=` do not change the held set.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use std::path::PathBuf;

use super::{CameraRig, PayneInput, PayneSim, pv};
use crate::math::{Vec3 as PVec, dir_pitch, dir_yaw, wrap_angle};

#[derive(Clone, Debug, PartialEq)]
enum Aim {
    Point(PVec),
    Torso(usize),
    Head(usize),
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Held {
    mv: [f32; 2],
    fire: bool,
    /// Fire taps (3 ticks down, 3 up) for semi-automatic weapons.
    mash: bool,
    reload: bool,
    jump: bool,
    crouch: bool,
    bullet_time: bool,
    shootdodge: bool,
    aim: Option<Aim>,
    yaw_rate: f32,
    pitch_rate: f32,
}

#[derive(Clone, Debug, PartialEq)]
enum Action {
    Held(Held),
    Shot(String),
    Quit,
    Cam(f32),
    /// Camera on enemy N (spawn order), or back on Max.
    Focus(Option<usize>),
    Weapon(usize),
    /// Gallery inspect camera: Some(None) = last board hit, Some(Some(n)) = board n, None = off.
    Inspect(Option<Option<usize>>),
    /// Spawn the combat wave.
    Combat,
    Painkiller,
    God(bool),
    /// Shootdodge slow motion on or off (the B key).
    Slowmo(bool),
    Hurt(f32),
    /// Teleport Max (x, z).
    Pos(f32, f32),
    /// Spawn one enemy: skin, x, z, yaw.
    Spawn(String, f32, f32, f32),
    /// Remove every target, enemy and corpse.
    Clear,
}

#[derive(Resource)]
pub struct PayneScript {
    /// (time, action), in time order.
    actions: Vec<(f32, Action)>,
    next: usize,
    cur: Held,
    pub shots_dir: PathBuf,
    /// Frames to wait after the last screenshot request before exiting.
    exit_in: Option<u32>,
}

impl PayneScript {
    pub fn parse(text: &str, shots_dir: impl Into<PathBuf>) -> Result<PayneScript, String> {
        let mut actions = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let err = |m: &str| format!("script line {}: {m}", n + 1);
            let mut w = line.split_whitespace();
            let t: f32 = w
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| err("starts with a time in seconds"))?;
            let mut held = Held::default();
            let mut sets_held = false;
            let mut extra = Vec::new();
            let nums =
                |s: &str| -> Option<Vec<f32>> { s.split(',').map(|v| v.parse().ok()).collect() };
            for tok in w {
                match tok {
                    "fwd" => held.mv[1] += 1.0,
                    "back" => held.mv[1] -= 1.0,
                    "left" => held.mv[0] -= 1.0,
                    "right" => held.mv[0] += 1.0,
                    "fire" => held.fire = true,
                    "mash" => held.mash = true,
                    "reload" => held.reload = true,
                    "jump" => held.jump = true,
                    "crouch" => held.crouch = true,
                    "shootdodge" => held.shootdodge = true,
                    "bt" => held.bullet_time = true,
                    "quit" => extra.push(Action::Quit),
                    "combat" => extra.push(Action::Combat),
                    "clear" => extra.push(Action::Clear),
                    "painkiller" => extra.push(Action::Painkiller),
                    _ => {
                        let Some((k, v)) = tok.split_once('=') else {
                            return Err(err(&format!("unknown token {tok}")));
                        };
                        match k {
                            "shot" => extra.push(Action::Shot(v.to_string())),
                            "inspect" => extra.push(Action::Inspect(match v {
                                "off" => None,
                                "last" => Some(None),
                                n => Some(Some(n.parse().map_err(|_| err("inspect=N|last|off"))?)),
                            })),
                            "god" => extra.push(Action::God(match v {
                                "on" => true,
                                "off" => false,
                                _ => return Err(err("god=on|off")),
                            })),
                            "slowmo" => extra.push(Action::Slowmo(match v {
                                "on" => true,
                                "off" => false,
                                _ => return Err(err("slowmo=on|off")),
                            })),
                            "hurt" => {
                                extra.push(Action::Hurt(v.parse().map_err(|_| err("hurt=damage"))?))
                            }
                            "spawn" => {
                                let w: Vec<&str> = v.split(',').collect();
                                let f = |i: usize| w.get(i).and_then(|x| x.parse::<f32>().ok());
                                match (w.first(), f(1), f(2)) {
                                    (Some(skin), Some(x), Some(z)) => {
                                        extra.push(Action::Spawn(
                                            skin.to_string(),
                                            x,
                                            z,
                                            f(3).unwrap_or(0.0),
                                        ));
                                    }
                                    _ => return Err(err("spawn=skin,x,z[,yaw]")),
                                }
                            }
                            "pos" => match nums(v).as_deref() {
                                Some([x, z]) => extra.push(Action::Pos(*x, *z)),
                                _ => return Err(err("pos=x,z")),
                            },
                            "weapon" => extra.push(Action::Weapon(
                                v.parse().map_err(|_| err("weapon=N needs a number"))?,
                            )),
                            "cam" if v.starts_with('e') && v[1..].parse::<usize>().is_ok() => {
                                extra.push(Action::Focus(v[1..].parse().ok()));
                            }
                            "cam" if v == "player" => extra.push(Action::Focus(None)),
                            "cam" => extra.push(Action::Cam(match v {
                                "front" => std::f32::consts::PI,
                                "side" => std::f32::consts::FRAC_PI_2,
                                "back" | "default" => 0.0,
                                _ => return Err(err("cam= is front, side, back or default")),
                            })),
                            "yaw" => {
                                held.yaw_rate = v.parse().map_err(|_| err("yaw=rad/s"))?;
                            }
                            "pitch" => {
                                held.pitch_rate = v.parse().map_err(|_| err("pitch=rad/s"))?;
                            }
                            "aim" => {
                                held.aim = Some(if let Some(i) = v.strip_prefix('t') {
                                    Aim::Torso(i.parse().map_err(|_| err("aim=tN"))?)
                                } else if let Some(i) = v.strip_prefix('h') {
                                    Aim::Head(i.parse().map_err(|_| err("aim=hN"))?)
                                } else {
                                    match nums(v).as_deref() {
                                        Some([x, y, z]) => Aim::Point(PVec::new(*x, *y, *z)),
                                        _ => return Err(err("aim=x,y,z")),
                                    }
                                });
                            }
                            _ => return Err(err(&format!("unknown token {tok}"))),
                        }
                        sets_held |= matches!(k, "yaw" | "pitch" | "aim");
                    }
                }
                sets_held |= matches!(
                    tok,
                    "fwd"
                        | "back"
                        | "left"
                        | "right"
                        | "fire"
                        | "mash"
                        | "reload"
                        | "jump"
                        | "crouch"
                        | "shootdodge"
                        | "bt"
                );
            }
            if sets_held || extra.is_empty() {
                actions.push((t, Action::Held(held)));
            }
            actions.extend(extra.into_iter().map(|a| (t, a)));
        }
        actions.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(PayneScript {
            actions,
            next: 0,
            cur: Held::default(),
            shots_dir: shots_dir.into(),
            exit_in: None,
        })
    }
}

/// Fixed tick: apply due actions, write the held input and the look that steers the aim.
pub fn drive(
    script: Option<ResMut<PayneScript>>,
    mut sim: ResMut<PayneSim>,
    mut combat: ResMut<super::PayneCombat>,
    mut input: ResMut<PayneInput>,
    mut cam: ResMut<CameraRig>,
    mut view: ResMut<super::GalleryView>,
) {
    let Some(mut s) = script else { return };
    let now = sim.0.time as f32 + 1e-4;
    while let Some((t, a)) = s.actions.get(s.next).cloned() {
        if t > now {
            break;
        }
        s.next += 1;
        match a {
            Action::Held(h) => s.cur = h,
            Action::Cam(o) => {
                cam.orbit = o;
                cam.focus = None;
            }
            Action::Focus(f) => cam.focus = f,
            Action::Weapon(n) => input.latch.select = Some(n),
            Action::Inspect(i) => {
                view.inspect = i.is_some();
                view.board = i.flatten();
            }
            Action::Combat => {
                super::spawn_wave(&mut sim.0);
                combat.active = true;
            }
            Action::Spawn(skin, x, z, yaw) => {
                sim.0.spawn_enemy(PVec::new(x, 0.0, z), yaw, &skin);
                combat.active = true;
            }
            Action::Clear => {
                sim.0.targets.clear();
                sim.0.corpses.clear();
                sim.0.projectiles.clear();
            }
            Action::Painkiller => input.latch.painkiller = true,
            Action::God(on) => {
                combat.god = on;
                if !on {
                    sim.0.player.immortal = false;
                }
            }
            Action::Slowmo(on) => sim.0.tuning.shootdodge_slowmo = if on { 1.0 } else { 0.0 },
            Action::Hurt(damage) => {
                let p = sim.0.player.pos;
                let from = sim
                    .0
                    .targets
                    .iter()
                    .find(|t| t.alive())
                    .map_or(PVec::new(0.0, 0.0, -1.0), |t| t.pos);
                let dir = (p - from).horizontal().normalize_or_zero();
                sim.0.damage_player(damage, dir);
            }
            Action::Pos(x, z) => {
                sim.0.player.pos = PVec::new(x, sim.0.player.pos.y, z);
            }
            Action::Shot(_) | Action::Quit => {}
        }
    }
    let h = s.cur.clone();
    input.held = crate::player::Input {
        move_axis: h.mv,
        fire: h.fire || (h.mash && (sim.0.time * 60.0).round() as i64 / 3 % 2 == 0),
        reload: h.reload,
        jump: h.jump,
        crouch: h.crouch,
        bullet_time: h.bullet_time,
        shootdodge: h.shootdodge,
        ..default()
    };
    let dt = crate::Sim::DT;
    let mut look = [h.yaw_rate * dt, h.pitch_rate * dt];
    let p = &sim.0.player;
    let goal = h.aim.as_ref().and_then(|a| match a {
        Aim::Point(p) => Some(*p),
        Aim::Torso(i) => sim
            .0
            .targets
            .get(*i)
            .map(|t| t.to_world(PVec::new(0.0, 1.2, 0.0))),
        Aim::Head(i) => sim
            .0
            .targets
            .get(*i)
            .map(|t| t.to_world(PVec::new(0.0, 1.66, 0.0))),
    });
    if let Some(g) = goal {
        // Steer the crosshair ray, which starts at the camera, onto the point.
        // With a camera on an enemy or orbiting Max the crosshair ray is off, so steer from his chest.
        let eye = if cam.focus.is_some() || cam.orbit != 0.0 {
            sim.0.player.chest(&sim.0.tuning)
        } else {
            pv(cam.position())
        };
        let d = g - eye;
        look[0] += wrap_angle(dir_yaw(d) - p.yaw).clamp(-0.3, 0.3);
        look[1] += (dir_pitch(d) - p.pitch).clamp(-0.3, 0.3);
    }
    input.look[0] += look[0];
    input.look[1] += look[1];
}

/// Update: request screenshots when their time comes, then quit.
pub fn shots(
    mut commands: Commands,
    script: Option<ResMut<PayneScript>>,
    sim: Res<PayneSim>,
    mut exit: MessageWriter<AppExit>,
    mut taken: Local<usize>,
    mut done: Local<bool>,
) {
    let Some(mut s) = script else { return };
    let now = sim.0.time as f32;
    if let Some(n) = s.exit_in.as_mut() {
        if *n == 0 {
            exit.write(AppExit::Success);
        } else {
            *n -= 1;
        }
        return;
    }
    let mut last = 0.0f32;
    let mut quit = None;
    for (t, a) in &s.actions {
        last = last.max(*t);
        if matches!(a, Action::Quit) {
            quit = Some(*t);
        }
    }
    let list: Vec<(f32, String)> = s
        .actions
        .iter()
        .filter_map(|(t, a)| match a {
            Action::Shot(n) => Some((*t, n.clone())),
            _ => None,
        })
        .collect();
    while *taken < list.len() && list[*taken].0 <= now {
        let path = s.shots_dir.join(format!("{}.png", list[*taken].1));
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        *taken += 1;
    }
    let end = quit.unwrap_or(last + 0.2);
    if now >= end && *taken >= list.len() && !*done {
        *done = true;
        s.exit_in = Some(45);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_held_state_and_commands() {
        let s = PayneScript::parse(
            "0.0 fwd\n0.5 shot=a # comment\n1.0 fire aim=t2 cam=front\n1.5\n2.0 quit\n",
            "x",
        )
        .unwrap();
        // fwd; shot (no held change); held for fire+aim; cam; bare time = release; quit
        let kinds: Vec<&Action> = s.actions.iter().map(|a| &a.1).collect();
        assert!(matches!(kinds[0], Action::Held(h) if h.mv == [0.0, 1.0]));
        assert!(matches!(kinds[1], Action::Shot(n) if n == "a"));
        assert!(matches!(kinds[2], Action::Held(h) if h.fire && h.aim == Some(Aim::Torso(2))));
        assert!(matches!(kinds[3], Action::Cam(_)));
        assert!(matches!(kinds[4], Action::Held(h) if *h == Held::default()));
        assert!(matches!(kinds[5], Action::Quit));
        assert!(PayneScript::parse("0.0 nonsense", "x").is_err());
        let c = PayneScript::parse("0.0 combat god=on pos=1,-2\n1.0 painkiller hurt=30\n", "x")
            .unwrap();
        let kinds: Vec<&Action> = c.actions.iter().map(|a| &a.1).collect();
        assert!(matches!(kinds[0], Action::Combat));
        assert!(kinds.contains(&&Action::God(true)));
        assert!(kinds.contains(&&Action::Pos(1.0, -2.0)));
        assert!(kinds.contains(&&Action::Hurt(30.0)));
        assert!(PayneScript::parse("0.0 god=maybe", "x").is_err());
    }
}
