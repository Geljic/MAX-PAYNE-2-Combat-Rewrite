# SKD (skin data)

Same container as KF2 (see `kf2.md`), holding one skin chunk `0x00010014`. Reader: `tools/extract/src/skd.rs`.
Reference: max_payne_ldb_importer specs. Verified on 69 retail MP2 `.skd` files (skin v1).

A skin says which meshes it deforms, which bones exist, and per vertex which bones influence it with what weight.
Bone indices point into the skin's own `skeleton_objects` list; vertices are indexed in the mesh named in
`skin_objects` (the character model, e.g. `Boozehound_A_L0.skd` skins `Boozehound_A`, 1981 vertices, 82 bones,
up to 3 influences per vertex).

## Version 0
Three lists of (vertex uint, bone uint, weight float), one per influence slot, merged per vertex by us; then
skin object names, skeleton object names, then count + vec3s of unknown meaning.

## Version 1 (what MP2 ships)
Each array is preceded by one raw byte (meaning unknown, ignored), then a counted list: skin object names;
skeleton object names; `vertex_indices`; `bones_per_vertex`; bone list; weights; vertex count per primitive;
start vertex per primitive. Vertex `i` has `bones_per_vertex[i]` influences taken consecutively from the bone list
and weight list. Our reader treats `vertex_indices` as list offsets when they equal the running sum of the counts
(vertex id = position), otherwise as vertex ids. In the retail files the first case applies.

## Skeleton
The bone hierarchy and bind pose live in mesh chunks of a skeleton `.kf2` (e.g. `skeletons/male/skeleton_male.kf2`,
86 bones): every mesh node is a bone with `name`, `parent_name` and `object_to_parent` bind transform.
`Skeleton::from_meshes` builds the tree.
