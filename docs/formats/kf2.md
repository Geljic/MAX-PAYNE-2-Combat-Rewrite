# KF2 (Max Payne 1/2 container: meshes, animation, cameras)

Reference: the format specs of max_payne_ldb_importer (GPL-3, read only). Reader: `tools/extract/src/kf2.rs`.
Verified on all 2,128 `.kf2`/`.skd` files extracted from the retail MP2 install (all parse, every chunk size matches).

## Primitives
Little-endian. Values are `{tag}{data}`:
`0x02` int32, `0x03` uint32, `0x09` f32, `0x0E` bool(u8), `0x0D` string = tag + int length + bytes (Latin-1),
`0x16` vec3, `0x1A` matrix4x3 (12 floats). Narrow ints exist (`0x11` u8, `0x10` u16, `0x0F` u24, `0x14` i8,
`0x13` i16, `0x12` i24), `0x26` is a half float. The reader accepts every int tag wherever an int is expected.
Some arrays (geometry v1, UV v1, polygons v1) hold untagged raw floats or u16 instead.

## Chunks
Header, 13 bytes: `0x0C`, id u32, version u32, size u32. **Size counts the whole chunk including the 13 header
bytes** (the upstream spec says "data size"; real files say otherwise). Chunks follow each other to end of file;
sub-chunks (node, animation, geometry...) are nested the same way inside their parent.

| Id | Chunk | Handled |
|---|---|---|
| 0x0001000F | Material list | yes (v0) |
| 0x00010005 | Mesh (bone node + geometry), see `kfs.md` | yes (v1, v2) |
| 0x00010014 | Skin, see `skd.md` | yes (v0, v1) |
| 0x00010012 | Keyframe animation | yes (v0-v5) |
| 0x00010001 | Camera | yes (v0) |
| 0x00010015 | Environment | yes (v0) |
| 0x00010002-4, 16, 17-19 | Lights, helper, light animations | skipped, noted |

## Keyframe animation (one chunk per animated node)
Fields in order: animation sub-chunk `0x00010013` v0 (`object_name`, `fps`, `is_looping`), `parent_name`,
`use_loop_interpolation`, `total_keyframes`, `key_count`, then `key_count` x (`frame` uint, matrix4x3).
v1+: visibility count + (`frame`, float). v2+: `loop_to_frame`. v3+: interpolation method int. v4+: maintain-scaling bool.
Frame count: v0-v4 store one less than the real number of frames, so add 1; v5 stores it directly.

A file is one clip: one track per bone (e.g. `ShootDodge_Forward.kf2`: 36 tracks, 30 fps, 51 frames).
`*_mov.kf2` files hold a single `MovementDummy` track carrying root motion.
Frame rates differ per file (30 or 50 seen). Clip duration in our reader = last frame index / fps.

## Transforms and units
`matrix4x3`: four rows of three floats; rows 0-2 are the axes (rotation x scale), row 3 is the translation, read as
row vectors (`v' = v * M`). Key transforms are relative to the parent bone. Translations are in metres
(the pelvis-height root sits at about y=0.97). Axes seem Y-up. Rotation convention is inferred from the layout and
not yet validated against in-game motion. `xform::rotation_quat` converts to a quaternion `[x,y,z,w]`.

## Open questions
- Meaning of `use_loop_interpolation`, `frame_to_frame_interpolation_method` values.
- Whether keyframes between key frames are linearly interpolated (assumed).
