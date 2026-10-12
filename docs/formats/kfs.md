# KFS (meshes and materials)

Same container as KF2 (see `kf2.md`). The mesh and material-list chunks also appear inside `.kf2` files (weapons,
skeletons, ragdolls). Reader: `tools/extract/src/kfs.rs`. Reference: max_payne_ldb_importer specs.
No `.kfs` files exist in the MP2 install; the same chunks were verified in `.kf2` files.

## Mesh chunk (0x00010005, v1 or v2)
Sub-chunks in any order, any may be missing: node `0x00010000` (v0/v1), geometry `0x00010006`, polygons `0x00010007`,
smoothing `0x0001000B` (v1 meshes), polygon material `0x0001000C`, UV mapping `0x0001000E` (one per layer),
reference to data `0x0001001A`. A mesh ends where the next chunk is not one of these.

| Sub-chunk | v0 (older) | v1 (MP2) |
|---|---|---|
| Node | name, parent name, matrix4x3 to parent, has_parent | + user string |
| Geometry | tagged vec3 positions | raw positions, raw normals, vertices per primitive |
| Polygons | nested polygon chunks (n-gons, we fan-triangulate) | u16 index list (3 per triangle), triangles per primitive |
| Polygon material | names + material index per polygon | names only (primitive index = material) |
| UV mapping | per-polygon index lists + vec3 coordinates | raw uvw coordinates, shared vertex indexing |

Mesh v2 indexes vertices per primitive: add the sum of earlier primitives' vertex counts to each index. We do that,
so `Mesh::triangles` index straight into `positions`. Each primitive is one material (`triangle_material`).
Bones that carry no geometry are mesh chunks with only a node (the skeleton, see `skd.md`).

## Material list (0x0001000F v0)
Semicolon-separated texture directories, then materials (`0x00010010` v0-v2): flags, colour types, ambient/
diffuse/specular RGBA floats, up to five optional texture sub-chunks (`0x00010011` v1: name, mip maps, filtering,
file names, animation block when more than one file). The spec's "more than zero files" for the animation block
is read as "more than one", following the reference implementation; the opacity-texture flag follows the spec.
