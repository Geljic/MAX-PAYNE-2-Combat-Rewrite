# Character physics: gravity and jumps

Read 2026-10-11 from `X_PhysicalSimulationMFC.dll` (the character controller update, around
0x10019690) and `X_SharedDBMFC.dll` (`X_SharedDBLevel::getGravity`).

- **Gravity is per level.** It is a vector in `data/database/levels/levels.txt`. All 29 retail
  levels use `( 0, GRAVITY_VALUE, 0 )`, with `GRAVITY_VALUE` defined as −20 at the top of the file.
  The character is built with that value (`X_CharacterPhysicalProperties(float gravity)`). So Max
  falls at **20 m/s²**, about twice real gravity. Particle systems use 9.81 instead, which is a
  red herring for the character.
- **Airborne step:** each frame the vertical speed gains gravity × dt, and the position moves by
  the *average* of the old and new vertical speed times dt. That traces the ballistic arc exactly,
  and payne does the same.
- **Jumps:** `C_Jump(v)` sets the take-off speed in m/s, and the controller adds it to the vertical
  speed once (an "acknowledged jumping velocity" stops it being applied twice). With g = 20:

  | Move | Take-off | Apex | Airtime |
  |---|---|---|---|
  | Standing jump (Jump_Up, frame 9) | 7.5 m/s | 1.41 m | 0.75 s |
  | Forward dodge (frame 11) | 4 m/s | 0.40 m | 0.40 s |
  | Shootdodge (frame 12) | 5 m/s | 0.63 m | 0.50 s, landing around frame 27 at 30 fps (world time) |

  This agrees with the side dodges' animations, which imply g ≈ 20. The shootdodge pose lies flat
  from about frame 36, so the dive animation keeps sliding along the floor after touchdown.

payne gets the value from `[level default] gravity` in `payne.data`, which the extractor
generates from `levels.txt`. The fallback default is 20.

## Dives in the sim

- **Air window.** A dive's airborne frames run from its `C_Jump` frame to the frame where that arc
  lands under `gravity` (12 to 27 for the forward dive). The `_mov.kf2` root curve agrees: its
  flight section ends at frame 27, where the floor slide starts.
- **Still airborne at the landing frame** (off a ledge, or a jump dive): the clip holds the frame
  before the landing frame (26) and the dive keeps the window's average speed (5.2 m/s forward).
  On touchdown the clip jumps to the landing frame and plays the slide and the lie-down. An
  early touchdown (landing on something higher) also jumps ahead to the landing frame.
  `AnimRequest.lower_time` carries the held or mapped clip time.
- **Jump dive** (payne's own move; Remedy's HIGHSHOOTDODGE* slots have no animations): shootdodge
  pressed in `Air` enters the dive clip at the C_Jump frame, already airborne, with the current
  vertical speed plus `jump_dive_boost` (default 0). The messages before that frame run at once,
  except the C_Jump: the frame-0 `C_Reload`, and with `shootdodge_slowmo` on the frame-10 game-speed
  change. You get one per airtime. A fall that follows a dodge roll cannot jump dive, because the
  roll already has its own window (`can_shootdodge_frame`). Shootdodge during the jump wind-up,
  still on the ground, is an ordinary dive.
  From the apex of a standing jump (1.41 m) the fall takes 0.375 s, shorter than a ground dive's
  0.5 s flight. The jump dive still stays up longer overall (0.77 s from take-off), and it flies
  further than a ground dive only with forward momentum: a running jump dive covers 3.9 m in the
  air against 2.6 m.
