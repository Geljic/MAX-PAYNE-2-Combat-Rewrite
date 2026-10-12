//! Timed moves: an animation's length, root-motion distance and frame-tagged messages.
//!
//! MP2 drives much of its gameplay from messages placed on animation frames (`skins/*.h`,
//! `skeletons/male.txt`): the shootdodge slows the game at frame 10 with
//! `Gm_ChangeGameSpeed(0.2, 0.3)`, leaves the ground at frame 12 with `C_Jump(5)`, and speeds the
//! game back up at frame 30. The state machine plays these timelines in player time and reacts to
//! the messages, so loading the real animation data changes the timing without code changes.
//!
//! The defaults below repeat the retail messages (they are part of the behaviour); frame counts,
//! fps and distances not yet measured from the `_mov.kf2` files are ESTIMATEs.
//!
//! Root motion: with game data each move carries its `_mov.kf2` curve ([`MoveDef::root`], the
//! horizontal distance from the start at every frame), and the player moves by the curve's slope,
//! so a dive builds speed on take-off, flies fast and slides to a stop as the animation does.
//! Without a curve the move falls back to `distance` at a constant speed.

/// A message on an animation frame that the simulation acts on.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// `Gm_ChangeGameSpeed(speed, blend)`: blend the world speed to `speed` over `blend` real seconds.
    GameSpeed { speed: f32, blend: f32 },
    /// `C_Jump(v)`: leave the ground with upward speed `v` (scaled by `Tuning::jump_scale`).
    Jump(f32),
    /// `C_Reload()`: instantly reload one `reload_amount` into the current weapon.
    Reload,
    /// `Gm_EnableBulletTime(on)`: switch bullet time on or off.
    EnableBulletTime(bool),
    /// `Gm_EnableBulletTimeBlock(on)`: while on, bullet time cannot be toggled.
    BulletTimeBlock(bool),
    /// `C_EnableProjectileCollisions(on)`: while off, bullets pass through the player.
    ProjectileCollisions(bool),
    /// `C_SetDodgeImmortal(on)`: while on, the player takes no damage.
    Immortal(bool),
    /// Anything else (sounds, effects, footsteps): passed to the host as an event.
    Other(String),
}

impl Msg {
    /// Parse `Function arg arg...` as written in `payne.data` (`msg = <frame> <Function> <args>`).
    /// Function names are case-insensitive, like MP2's script.
    pub fn parse(func: &str, args: &[&str]) -> Msg {
        let num = |i: usize| args.get(i).and_then(|a| crate::data::parse_number(a));
        let flag = |i: usize| {
            args.get(i)
                .map(|a| matches!(a.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
        };
        // Receivers come as `Maxpayne_Gamemode->` in the game's script and `Gamemode.` in payne.data.
        let name = func
            .rsplit(['>', '.'])
            .next()
            .unwrap_or(func)
            .to_ascii_lowercase();
        match name.as_str() {
            "gm_changegamespeed" => match (num(0), num(1)) {
                (Some(speed), blend) => Msg::GameSpeed {
                    speed,
                    blend: blend.unwrap_or(0.0),
                },
                _ => Msg::Other(func.to_string()),
            },
            "c_jump" => num(0).map_or(Msg::Other(func.to_string()), Msg::Jump),
            "c_reload" => Msg::Reload,
            "gm_enablebullettime" => {
                flag(0).map_or(Msg::Other(func.to_string()), Msg::EnableBulletTime)
            }
            "gm_enablebullettimeblock" => {
                flag(0).map_or(Msg::Other(func.to_string()), Msg::BulletTimeBlock)
            }
            "c_enableprojectilecollisions" => {
                flag(0).map_or(Msg::Other(func.to_string()), Msg::ProjectileCollisions)
            }
            "c_setdodgeimmortal" => flag(0).map_or(Msg::Other(func.to_string()), Msg::Immortal),
            _ => Msg::Other(
                std::iter::once(func)
                    .chain(args.iter().copied())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimMsg {
    pub frame: f32,
    pub msg: Msg,
}

/// One timed move.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveDef {
    /// MP2 animation name without the `CHARANIM_` prefix, e.g. `SHOOTDODGEFORWARD`.
    pub name: String,
    pub frames: f32,
    pub fps: f32,
    /// Horizontal root-motion distance over the whole move (m). Used at a constant speed only when
    /// there is no `root` curve.
    pub distance: f32,
    /// Root-motion curve: horizontal distance from the start (m) at each frame `0, 1, ... n`,
    /// measured from the `_mov.kf2` by `payne-extract measure`. Empty: constant speed.
    pub root: Vec<f32>,
    /// Overall travel direction in the clip's own frame `(x right, z forward)`, normalised, when
    /// measured (forward dive `0 1`, left dive `-1 0`). The player uses it for moves that have no
    /// stick direction of their own (get-ups, the spin).
    pub root_dir: Option<[f32; 2]>,
    pub msgs: Vec<AnimMsg>,
}

impl MoveDef {
    pub fn new(name: &str, frames: f32, fps: f32, distance: f32, msgs: Vec<(f32, Msg)>) -> MoveDef {
        MoveDef {
            name: name.to_string(),
            frames,
            fps,
            distance,
            root: Vec::new(),
            root_dir: None,
            msgs: msgs
                .into_iter()
                .map(|(frame, msg)| AnimMsg { frame, msg })
                .collect(),
        }
    }
    /// Length in player seconds.
    pub fn duration(&self) -> f32 {
        if self.fps > 0.0 {
            self.frames / self.fps
        } else {
            0.0
        }
    }
    /// Time (s) of a frame of this move.
    pub fn time_of(&self, frame: f32) -> f32 {
        if self.fps > 0.0 {
            frame / self.fps
        } else {
            0.0
        }
    }
    /// Frame at time `t` (s).
    pub fn frame_at(&self, t: f32) -> f32 {
        t * self.fps
    }
    /// The same move with a root-motion curve (distance at each frame).
    pub fn with_root(mut self, root: Vec<f32>) -> MoveDef {
        if let Some(&last) = root.last() {
            self.distance = last;
        }
        self.root = root;
        self
    }
    /// Average root-motion speed over the whole move (m/s): the constant speed used without a curve.
    pub fn speed(&self) -> f32 {
        let d = self.duration();
        if d > 0.0 { self.distance / d } else { 0.0 }
    }
    /// Does this move travel at all?
    pub fn has_motion(&self) -> bool {
        self.root.last().map_or(self.distance, |&d| d) > 1e-4
    }
    /// Horizontal distance from the start at time `t` (s): the root curve interpolated linearly
    /// between frames, clamped to its ends; without a curve, `distance` spread evenly over the move.
    pub fn distance_at(&self, t: f32) -> f32 {
        if self.root.is_empty() {
            let d = self.duration();
            return if d > 0.0 {
                self.distance * (t / d).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
        let n = self.root.len();
        let f = self.frame_at(t).clamp(0.0, (n - 1) as f32);
        let a = (f.floor() as usize).min(n - 1);
        let b = (a + 1).min(n - 1);
        let k = f - a as f32;
        self.root[a] + (self.root[b] - self.root[a]) * k
    }
    /// Average horizontal speed between two times (m/s): `(distance_at(t1) - distance_at(t0)) / (t1 - t0)`.
    pub fn speed_between(&self, t0: f32, t1: f32) -> f32 {
        if t1 - t0 > 1e-6 {
            (self.distance_at(t1) - self.distance_at(t0)) / (t1 - t0)
        } else {
            0.0
        }
    }
    /// Speed over the last frame of the move: what is left when it ends (≈0 for a dive that slides
    /// to a stop; the constant speed without a curve).
    pub fn end_speed(&self) -> f32 {
        if self.root.len() < 2 {
            return self.speed();
        }
        let d = self.time_of((self.root.len() - 1) as f32);
        self.speed_between((d - 1.0 / self.fps.max(1e-6)).max(0.0), d)
    }
    /// The `C_Jump` frame and take-off speed, if the move leaves the ground.
    pub fn jump(&self) -> Option<(f32, f32)> {
        self.msgs.iter().find_map(|m| match m.msg {
            Msg::Jump(v) => Some((m.frame, v)),
            _ => None,
        })
    }
    /// The airborne window `(take-off time, landing time)` in clip seconds. The landing frame is
    /// where the `C_Jump(v)` arc comes back to take-off height under `gravity` (with the C_Jump
    /// argument times `jump_scale` as the take-off speed): for the forward dive C_Jump(5) at frame
    /// 12 under MP2's 20 m/s² lands at frame 27, which is also where the `_mov.kf2` curve turns
    /// from flight (≈5.2 m/s) into the floor slide. Chosen over the pelvis height in the clip,
    /// which reaches the floor only at frame 36 (the pose is animated, not simulated).
    pub fn air_window(&self, gravity: f32, jump_scale: f32) -> Option<(f32, f32)> {
        let (frame, v) = self.jump()?;
        let t0 = self.time_of(frame);
        let air = 2.0 * v * jump_scale / gravity.max(1e-4);
        Some((t0, t0 + air))
    }
    /// Average horizontal speed over the airborne window: what a dive keeps while it is still in
    /// the air after its animation would have landed.
    pub fn air_speed(&self, gravity: f32, jump_scale: f32) -> Option<f32> {
        let (t0, t1) = self.air_window(gravity, jump_scale)?;
        Some(self.speed_between(t0, t1))
    }
    /// Messages whose time lies in [t0, t1): frame 0 fires on the first step.
    pub fn msgs_between(&self, t0: f32, t1: f32) -> impl Iterator<Item = &Msg> {
        self.msgs
            .iter()
            .filter(move |m| {
                let t = self.time_of(m.frame);
                t >= t0 && t < t1
            })
            .map(|m| &m.msg)
    }
    /// Time of the first message matching `pred`, if any.
    pub fn first_time(&self, pred: impl Fn(&Msg) -> bool) -> Option<f32> {
        self.msgs
            .iter()
            .find(|m| pred(&m.msg))
            .map(|m| self.time_of(m.frame))
    }
}

/// Every timed move the player uses. Direction-specific moves are `[forward, back, left, right]`.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveSet {
    /// Shootdodge in each direction. Only the forward one carries messages in the retail data,
    /// so the default uses its timeline for all four (ASSUMPTION).
    pub shootdodge: [MoveDef; 4],
    /// Getting up after a shootdodge, per dive direction.
    pub getup: [MoveDef; 4],
    /// The dodge roll (crouch while moving), per direction.
    pub dodge: [MoveDef; 4],
    /// Standing jump (Jump_Up) and running jump (Jump_Forward).
    pub jump_up: MoveDef,
    pub jump_forward: MoveDef,
    /// Bullet-time spin reload (`CHARANIM_WHIRLLEFT`, bt2_reload_handgun.kf2).
    pub spin: MoveDef,
}

impl Default for MoveSet {
    fn default() -> Self {
        use Msg::*;
        // skins/Male_Player.h CHARANIM_SHOOTDODGEFORWARD. 51 frames at 30 fps over 5.80 m:
        // measured from ShootDodge_Forward_mov.kf2.
        let sd = |name: &str| {
            MoveDef::new(
                name,
                51.0,
                30.0,
                5.8,
                vec![
                    (0.0, Reload),
                    (
                        10.0,
                        GameSpeed {
                            speed: 0.2,
                            blend: 0.3,
                        },
                    ),
                    (12.0, Jump(5.0)),
                    (12.0, EnableBulletTime(false)),
                    (12.0, BulletTimeBlock(true)),
                    (
                        30.0,
                        GameSpeed {
                            speed: 1.0,
                            blend: 0.5,
                        },
                    ),
                ],
            )
        };
        // CHARANIM_SHOOTDODGE*_GETUP: messages from Male_Player.h; 30 frames is an ESTIMATE.
        let getup = |name: &str| {
            MoveDef::new(
                name,
                30.0,
                30.0,
                0.0,
                vec![
                    (
                        0.0,
                        GameSpeed {
                            speed: 1.0,
                            blend: 0.5,
                        },
                    ),
                    (20.0, BulletTimeBlock(false)),
                ],
            )
        };
        // Dodges: messages from Male_Player.h (left/right/back) and male.txt (forward);
        // 36 frames over 3.0 m is an ESTIMATE until Dodge_*_mov.kf2 is measured.
        let dodge_fwd = MoveDef::new("DODGEFORWARD", 36.0, 30.0, 3.0, vec![(11.0, Jump(4.0))]);
        let side = |name: &str| {
            MoveDef::new(
                name,
                36.0,
                30.0,
                3.0,
                vec![
                    (
                        0.0,
                        GameSpeed {
                            speed: 0.7,
                            blend: 0.0,
                        },
                    ),
                    (0.0, ProjectileCollisions(false)),
                    (8.0, Jump(4.0)),
                    (
                        25.0,
                        GameSpeed {
                            speed: 1.0,
                            blend: 0.5,
                        },
                    ),
                    (30.0, ProjectileCollisions(true)),
                ],
            )
        };
        let dodge_back = MoveDef::new(
            "DODGEBACKWARD",
            30.0,
            30.0,
            2.5,
            vec![
                (
                    0.0,
                    GameSpeed {
                        speed: 0.7,
                        blend: 0.0,
                    },
                ),
                (0.0, ProjectileCollisions(false)),
                (11.0, Jump(2.0)),
                (
                    17.0,
                    GameSpeed {
                        speed: 1.0,
                        blend: 0.5,
                    },
                ),
                (19.0, ProjectileCollisions(true)),
            ],
        );
        MoveSet {
            shootdodge: [
                sd("SHOOTDODGEFORWARD"),
                sd("SHOOTDODGEBACKWARD"),
                sd("SHOOTDODGELEFT"),
                sd("SHOOTDODGERIGHT"),
            ],
            getup: [
                getup("SHOOTDODGEFORWARD_GETUP"),
                getup("SHOOTDODGEBACKWARD_GETUP"),
                getup("SHOOTDODGELEFT_GETUP"),
                getup("SHOOTDODGERIGHT_GETUP"),
            ],
            dodge: [dodge_fwd, dodge_back, side("DODGELEFT"), side("DODGERIGHT")],
            // male.txt: Jump_Up C_Jump(7.5) at frame 9, Jump_Forward at frame 0. Frame counts ESTIMATE.
            jump_up: MoveDef::new("JUMP", 20.0, 30.0, 0.0, vec![(9.0, Jump(7.5))]),
            jump_forward: MoveDef::new("JUMPFORWARD", 20.0, 30.0, 0.0, vec![(0.0, Jump(7.5))]),
            // male.txt CHARANIM_WHIRLLEFT messages; 45 frames is an ESTIMATE.
            spin: MoveDef::new(
                "WHIRLLEFT",
                45.0,
                30.0,
                0.0,
                vec![
                    (0.0, Immortal(true)),
                    (38.0, Reload),
                    (38.0, Immortal(false)),
                ],
            ),
        }
    }
}

impl MoveSet {
    /// Every move, for replacing by name from data.
    pub fn all_mut(&mut self) -> impl Iterator<Item = &mut MoveDef> {
        self.shootdodge
            .iter_mut()
            .chain(self.getup.iter_mut())
            .chain(self.dodge.iter_mut())
            .chain([&mut self.jump_up, &mut self.jump_forward, &mut self.spin])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_retail_messages() {
        assert_eq!(
            Msg::parse("Gm_ChangeGameSpeed", &["0.2", "0.3"]),
            Msg::GameSpeed {
                speed: 0.2,
                blend: 0.3
            }
        );
        assert_eq!(
            Msg::parse("Maxpayne_Gamemode->Gm_Changegamespeed", &["1.0", "0.5"]),
            Msg::GameSpeed {
                speed: 1.0,
                blend: 0.5
            }
        );
        assert_eq!(Msg::parse("C_Jump", &["5"]), Msg::Jump(5.0));
        assert_eq!(
            Msg::parse("gm_enablebullettimeBlock", &["false"]),
            Msg::BulletTimeBlock(false)
        );
        assert!(matches!(
            Msg::parse("A_Play3DSound", &["misc", "Dodge"]),
            Msg::Other(_)
        ));
    }

    #[test]
    fn frame_zero_fires_first_step() {
        let m = MoveSet::default();
        let first: Vec<_> = m.shootdodge[0].msgs_between(0.0, 1.0 / 60.0).collect();
        assert_eq!(first, vec![&Msg::Reload]);
        assert!((m.shootdodge[0].duration() - 1.7).abs() < 1e-5);
    }

    #[test]
    fn root_curve_gives_the_speed_profile() {
        let sd = MoveSet::default().shootdodge[0].clone();
        // No curve: constant speed.
        assert!((sd.speed_between(0.1, 0.2) - sd.speed()).abs() < 1e-4);
        assert!((sd.distance_at(sd.duration() * 0.5) - sd.distance * 0.5).abs() < 1e-4);
        assert!((sd.end_speed() - sd.speed()).abs() < 1e-5);
        // A curve: interpolated between frames, slope as speed.
        let m = MoveDef::new("X", 4.0, 10.0, 0.0, vec![(1.0, Msg::Jump(5.0))])
            .with_root(vec![0.0, 1.0, 3.0, 3.0]);
        assert_eq!(m.distance, 3.0);
        assert!((m.distance_at(0.15) - 2.0).abs() < 1e-5);
        assert!((m.speed_between(0.1, 0.2) - 20.0).abs() < 1e-3);
        assert!(m.end_speed().abs() < 1e-5);
        assert!((m.distance_at(9.0) - 3.0).abs() < 1e-6);
        // C_Jump(5) at frame 1 under g = 20 lands 0.5 s later.
        let (t0, t1) = m.air_window(20.0, 1.0).unwrap();
        assert!((t0 - 0.1).abs() < 1e-6 && (t1 - 0.6).abs() < 1e-6);
    }
}
