# Values measured from the game's animations

`payne-extract measure <payne.data> <anims.bin>` (`tools/extract/src/measure.rs`) reads the baked clips
and writes what it can measure into `payne.data`, between `# measured by payne-extract` and
`# end measured by payne-extract` comments. Running it again replaces those blocks. The
character values go at the end of `[character MaxPayne]` (Tuning field names), the move values right
after the header of each `[anim NAME]` section (`frames`, `fps`, `distance`). Run it after `bake` and
`generate`; `generate` rewrites `payne.data` and drops the blocks.

Conventions: Y up, metres, the model's feet at y = 0 (the ankle bones sit at 0.10). "Root travel" is
the last frame of a clip's movement track (`RootTrack`), horizontal part only. A looping clip's speed is
its travel over one cycle. Heights are model-space bone positions sampled at integer frames, and bone
centres, not surfaces.

## Emitted: `[character MaxPayne]`

| Key | Measured | Old default | Clip(s) and method |
|---|---|---|---|
| `run_speed` | 5.25 | 5.25 | RUNFORWARD: 4.2 m of root travel in 40 frames at 50 fps (0.8 s). Unchanged. |
| `strafe_speed` | 5.25 | 4.6 | RUNLEFT and RIGHT: 4.2 m in 0.8 s each, averaged. Diagonals (RUNFORWARDLEFT etc.) are 2.97 m on each axis, 4.2 m total, so also 5.25. |
| `backpedal_speed` | 3.75 | 4.0 | RUNBACKWARD: 2.5 m in 40 frames at 60 fps (0.667 s). Backward diagonals are the same 3.75. |
| `crouch_top` | 1.475 | 1.2 | Head bone in CROUCH once the pose is held (pelvis within 5 cm of its lowest): 1.138. Plus the standing gap between the head bone (STAND, 1.613) and the game's `CapsuleTop` (1.95), 0.337. The gap is kept so the capsule stays as tall above the head as the game's. |
| `prone_top` | 0.557 | 0.6 | Highest bone (not the floor helper) in the last frame of each of the four SHOOTDODGE clips, the largest of them 0.437, plus 0.12 m for the thickness of the body (our margin, not from the game). |
| `muzzle_stand` | 1.610 | 1.4 | `Bip01 R Gun` height in SHOOTBERETTA frame 0 (the aim-level shooting pose, standing). It is the hand/gun bone, not the barrel tip; the barrel is forward of it, not higher. |
| `muzzle_side` | 0.183 | 0.22 | `Bip01 R Gun` x in the same pose (right of the body). |
| `muzzle_crouch` | 1.164 | 0.95 | The aiming clip animates the pelvis but not the legs, so it cannot be laid over CROUCH directly. Instead: gun height above the pelvis bone in SHOOTBERETTA (0.670) plus the held CROUCH pelvis height (0.494). |
| `muzzle_prone` | 0.194 | 0.4 | `Bip01 R Gun` height in the last frame of SHOOTDODGEFORWARD (lying on the front). The other dives end 0.27 to 0.37 m up (back, left). |
| `hard_land_time` | 0.633 s | 0.6 | LAND (Land.kf2, 20 frames at 30 fps, one-shot so 19/30 s). LAND tags `dummy_land_hard` on frame 0, so it is the hard landing. |
| `flinch_time` | 0.5 s | 0.3 | SMALLDAMAGE (GetHit_Small.kf2): 20 frames at 40 fps, a looping clip so its period is 0.5 s. Other hit clips for reference: VERYSMALL 0.3 s, MEDIUM 1.0 s (both loops), LARGE 1.7 s, VERYLARGE 2.675 s (one-shots). |
| `switch_time` | 0.533 s | 0.35 | GETWEAPON (PutGetGun_Split.kf2): 17 frames at 30 fps, 16/30 s. This is the draw only. HOLSTERWEAPON (PutGetGun.kf2) is 10/30 = 0.333 s more if the old weapon is put away first. |

## Emitted: `[anim NAME]` (`frames`, `fps`, `distance`)

`frames` is the clip's frame count and `fps` its rate, the same convention as the existing shootdodge
default (51 frames at 30 fps); message frames are frames of this clip. `distance` is the root travel.
The sim follows the root curve below; `distance / (frames / fps)` is only the fallback without one.

| Move | frames @ fps | distance (m) | Old default |
|---|---|---|---|
| SHOOTDODGEFORWARD | 51 @ 30 | 5.800 | 51 @ 30, 5.8 |
| SHOOTDODGEBACKWARD | 51 @ 30 | 5.134 | 5.8 (forward's) |
| SHOOTDODGELEFT | 51 @ 30 | 5.210 | 5.8 |
| SHOOTDODGERIGHT | 51 @ 30 | 5.156 | 5.8 |
| SHOOTDODGE{FORWARD,BACKWARD,LEFT,RIGHT}_GETUP | 26 @ 30 | no root | 30 @ 30 |
| DODGEFORWARD | 41 @ 30 | 5.560 | 36 @ 30, 3.0 |
| DODGEBACKWARD | 21 @ 30 | 2.721 | 30 @ 30, 2.5 |
| DODGELEFT, DODGERIGHT | 31 @ 30 | 5.243 | 36 @ 30, 3.0 |
| JUMP | 31 @ 40 | no root | 20 @ 30 |
| JUMPFORWARD | 17 @ 40 | 1.505 | 20 @ 30 |
| WHIRLLEFT (spin reload) | 39 @ 60 | no root | 45 @ 30 |

Notes: the four dodge diagonals have no section of their own in the move set (the sim picks forward,
back, left or right). The dive and the get-up are slow-motion clips; the numbers are in the clip's own
(player) time. WHIRLLEFT's `C_Reload` message sits on frame 38, the last frame of a 39-frame clip, so
the message frames are in the clip's own 60 fps frames, which fits the other clips. WALK clips are not
used by any Tuning field, but for reference: WALKFORWARD 1.83 m/s, WALKBACKWARD 1.12, WALKLEFT 1.71,
WALKRIGHT 1.71.

## Emitted: root-motion curves (`root`, `root_dir`)

For every clip in `anims.bin` that has a root track and a `[anim NAME]` section in `payne.data`:

- `root = d0 d1 ... dn`: horizontal distance from the start at each clip frame (n + 1 = `frames`
  values), in metres. Sampled from the root track at the clip's own frame times (the track can run at
  another rate) and taken as the magnitude of the horizontal offset, so side and back moves give the
  same growing curve as forward ones.
- `root_dir = x z`: the last root offset, normalised, in the clip's frame (x right, z forward):
  forward dive `0 1`, back `0 -1`, left `-1 0`, right `1 0`.

The sim moves timed moves by the curve's slope, `(distance_at(t1) - distance_at(t0)) / dt`, with
linear interpolation between frames (`MoveDef::distance_at`). For the forward shootdodge
(ShootDodge_Forward_mov.kf2) the curve is:

| frame | 0 | 6 | 12 | 15 | 18 | 24 | 27 | 30 | 33 | 36 | 42 | 50 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| m | 0 | 0.53 | 1.50 | 1.97 | 2.49 | 3.57 | 4.10 | 4.58 | 5.00 | 5.33 | 5.70 | 5.80 |

Take-off (frames 0 to 12) builds from 1.5 to 4.9 m/s, the flight (12 to 27) holds about 5.2 m/s,
and the floor slide (27 to 50) runs from 4.8 m/s down to 0. Frame 27 is exactly where C_Jump(5)
at frame 12 lands under MP2's gravity of 20 m/s² (`docs/behaviour/physics.md`). The sim therefore
takes the dive's landing frame from physics (`MoveDef::air_window`: C_Jump frame + 2 v / g × fps),
not from the pelvis height, which reaches the floor only at frame 36. The pose keeps settling for
those nine frames while the capsule is already sliding. Because the curve slides to a stop,
Prone starts with the speed of the last frame (0.04 m/s), and `prone_slide_decel` acts only
without data. Measured in `tests/root_motion.rs`: total 5.80 m, 4.10 m at touchdown on frame 27.0,
5.19 m/s in the air.

Cutscene clips (`CUSTOM*`) also get curves because their sections exist. That grows `payne.data`
from about 0.3 MB to 0.8 MB, which is harmless.

## Gravity and the jump: not emitted

Measured with `measure`'s `gravity evidence` lines. For each move that has a `C_Jump(v)`, the take-off
frame is the message frame and the landing frame is the first one after it where the pelvis (`Bip01`)
height is within a margin of the lowest it gets afterwards (the body is on the floor). Gravity that
lands a take-off at `v` m/s in that time is `2 v / t`. With a margin of 0.10 m:

| Move | take-off | lands | implied g (m/s²) |
|---|---|---|---|
| SHOOTDODGEFORWARD, C_Jump(5) | 12 | 36 | 12.5 |
| SHOOTDODGEBACKWARD | 12 | 32 | 15.0 |
| SHOOTDODGELEFT | 12 | 29 | 17.6 |
| SHOOTDODGERIGHT | 12 | 28 | 18.8 |
| DODGEFORWARD, C_Jump(4) | 11 | 27 | 15.0 |
| DODGELEFT, DODGERIGHT | 8 | 20 | 20.0 |
| DODGEBACKWARD, C_Jump(2) | 11 | none | the body is already on the floor |

The values spread from 12.5 to 20 (margins 0.05 and 0.15 move single clips by up to 5 and change none of
the picture). So the clips do not fit one gravity, and not 9.81 either: at 9.81 and the message
arguments as m/s, the forward dive would still be airborne at frame 42 while its pose is lying flat
from frame 36. The poses are animated to look right and the physics values were tuned by hand in the
game code, so nothing consistent can be recovered from the clips. Gravity and `jump_scale` stay ESTIMATE.
If you want the forward shootdodge (the move that matters) to land when its animation does, g near 12
with the C_Jump(5) argument as m/s does that; the dodges would then be 20 % to 40 % off.

JUMP (Jump_Up.kf2, C_Jump(7.5) at frame 9) is no help: the clip is 31 frames at 40 fps, so it ends
0.55 s after take-off, 0.2 s before the apex at 9.81 (0.76 s, 2.87 m), and has no landing; the Fall
loop takes over. JUMPFORWARD covers 1.505 m in its 0.425 s, about 3.5 m/s along the ground while
the sim steers at `airborne_speed` 2.5, which is the game's own figure. The apex the game intends
therefore cannot be measured from the animations. 2.87 m (g 9.81, v 7.5) remains an unverified guess,
and the old note that it looks too high still stands.

## Not measured, still ESTIMATE

| Field | Why |
|---|---|
| `crouch_speed` | No crouch-walk clip in the data: CROUCH is a held pose, and the walk and run clips are standing. The value is in the character code. |
| `land_time` | No soft-landing clip. FALL only tags `dummy_land_soft` on frame 7; LAND is the hard landing. |
| `step_up`, `snap_down`, `accel`, `decel`, `body_turn_rate`, `air_accel`, `jump_speed`, `jump_scale`, `gravity` | Character-controller code, not animation (the turn clips have no root yaw). |
| `pitch_min`, `pitch_max` | Camera code. |
| `prone_slide_decel` | Not needed with data: the dive follows its root curve, which slides to a stop by the last frame, so Prone starts at about 0. The value only shapes the constant-speed fallback. |
| `prone_twist`, `prone_turn_rate`, `roll_min_speed`, spreads, recoil, ragdoll values, `projectile_lifetime` | Weapon, physics or code values, not in any `.kf2`. |
| Head height for the camera | No Tuning field. For reference: the head bone is 1.613 m (STAND) and 1.138 m (CROUCH). |
