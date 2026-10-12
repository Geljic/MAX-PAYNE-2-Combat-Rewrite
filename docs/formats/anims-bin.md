# anims.bin (payne's baked animation format)

Written by `payne-extract bake <mp2-files dir> <out>` (`tools/extract/src/bake.rs`), read by `src/clips.rs`.
Generated from the player's own files, gitignored (`assets/generated/anims.bin`). Little-endian throughout.
Strings are `u16 length` + UTF-8 bytes.

## Layout (version 2)
```
magic        8 bytes  "PAYNEANM"
version      u32      2
bone_count   u32
bone[bone_count]:
  name       string
  parent     i32      -1 for the root; always lower than the bone's own index
  bind_t     3 x f32  bind-pose local translation (metres)
  bind_q     4 x f32  bind-pose local rotation, quaternion x y z w
clip_count   u32
clip[clip_count]:
  name       string   game name without CHARANIM_, upper case (SHOOTDODGEFORWARD, 2_RUNFORWARD)
  source     string   source file name (Run_Forward.kf2)
  fps        f32
  frame_count u32
  looping    u8
  track_count u32
  bone_index[track_count]  u16   skeleton bones this clip animates, ascending
  t_animated[track_count]  u8    1 if the track's translation changes over time
  for each track with t_animated == 0: constant translation, 3 x f32
  rotations  frame_count * track_count * 4 x i16   frame-major, quaternion x y z w, value / 32767
  translations: frame_count * (number of animated tracks) * 3 x f32  frame-major, in track order
  has_root   u8
  if has_root: root_fps f32, root_frames u32, root_frames x (x, y, z, yaw) f32
```
Rotations are dense (one per frame) and quantised to 16 bits (error about 3e-5, far below a millimetre on a
limb). Translations are stored per frame only for bones that move relative to their parent (the pelvis and a
few helpers); the rest store one value.

## Conventions
- Y up, metres, **+Z is the character's forward** (see handedness below).
- Handedness: the data is left-handed (Direct3D style: X right, Y up, Z forward as seen from behind the
  character, so the character's left side is -X). Checked in the stick-figure renders: facing +Z, the bones
  named "L" sit on -X. A right-handed engine (Bevy faces -Z) negates Z (and conjugates rotations accordingly)
  or turns the character 180 degrees about Y and accepts a mirrored model; pick one in the host.
- Rotation: the KF2 matrix is a row-vector matrix (`v' = v * M`, translation in row 3). The baker converts it to
  a quaternion that rotates column vectors (`v' = q v q*`). Confirmed on real data: with that reading the feet
  of POSE/STAND/RUN sit at y = 0 and the head at 1.6 m, and the shootdodge dives flat with the head leading.
- Joints compose `world = parent_world * local` (`Pose::model_space`). Bone offsets are along local X
  (Biped style), so bind translations are mostly `(length, 0, 0)`.
- Bone scale in the source is 1 (within 1e-6) and is dropped.
- Interpolation: sparse source keys are lerped (translation) and slerped (rotation) when baking; before the
  first / after the last key the value holds. Playback interpolates between the baked frames the same way.
- Time: a looping clip's period is `frame_count / fps` (the last frame blends back into frame 0); a one-shot
  lasts `(frame_count - 1) / fps` and clamps at its last frame.
- Hierarchy: four retail clips (RUNLEFT, RUNRIGHT, SHOOTHIGHMP5, SHOOTLOWMP5, plus their `2_` variants) hang the
  thighs from the pelvis and jackets from the clavicle, unlike the skeleton file (thighs under Spine). The baker
  re-expresses their locals under the skeleton's parents, so every clip fits one skeleton.

## Root motion
Walking, running and dodging move the character through a separate `MovementDummy` / `Footsteps` track in the
clip's `_mov.kf2` file. The `Bip01` bone track itself stays near the origin (it carries bounce and lean, not
travel). So a host can:
- keep the character in place: ignore `root` (use the pose as sampled), or
- drive it: add `root_motion(t0, t1)` each tick to the character's position, rotating the translation by the
  character's yaw at the start of the clip; add the yaw delta to its heading.
Root frames are offsets from the first frame (frame 0 is zero); yaw is radians, positive from +Z towards +X,
unwrapped across frames. `root_fps` can differ from the clip's `fps`, and the root track can have one frame more
than the clip (RUNFORWARD: 40 clip frames, 41 root frames, one 0.8 s cycle covering 4.20 m).
Looping clips add whole cycles of root travel.

## Which clips
Everything `payne.data` lists as `[anim NAME]` (`file =`, optional `movement =`, paths relative to
`data/database/`) when that file sits next to the output, plus every animation in `skins/Male_Player.h`, plus
`STAND` / `2_STAND`. Merged by name. Currently 481 clips, 39 MB.

## Known non-rigid bones
Bone lengths stay within 1% of the bind pose for the spine, neck, head, arms, calves and feet in every clip
(worst 0.72%). The animators move some attachment bones on purpose: hips (thigh offset up to 4%), shoulders,
jackets, back, gun bones (up to 8 cm in melee), clavicles, fingers in two death clips, face bones and the
`Bip01 Footsteps` floor helper.
