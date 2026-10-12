//! Skin chunks (the content of `.skd` files) and the skeleton built from mesh
//! nodes. See `docs/formats/skd.md`.

use std::collections::{BTreeMap, HashMap};

use crate::kfs::Mesh;
use crate::reader::{ChunkHeader, Reader, Result};
use crate::xform::Mat4x3;

pub const ID_SKIN: u32 = 0x0001_0014;

/// Bone influences of one vertex.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinVertex {
    pub vertex: u32,
    /// Indices into [`Skin::skeleton_objects`].
    pub bones: Vec<u32>,
    pub weights: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skin {
    pub version: u32,
    /// Names of the meshes this skin deforms.
    pub skin_objects: Vec<String>,
    /// Names of the bones; bone indices in `vertices` point here.
    pub skeleton_objects: Vec<String>,
    pub vertices: Vec<SkinVertex>,
    /// Version 0 only: trailing vectors of unknown meaning.
    pub unknown_vectors: Vec<[f32; 3]>,
}

/// Version 1 stores a raw byte in front of every array; its meaning is not
/// documented, so it is read and ignored.
fn lead_byte(r: &mut Reader, v1: bool, what: &str) -> Result<()> {
    if v1 { r.u8_raw(what).map(|_| ()) } else { Ok(()) }
}

fn int_array(r: &mut Reader, what: &str) -> Result<Vec<u32>> {
    let n = r.count(what, 2)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.uint(what)?);
    }
    Ok(v)
}

fn string_array(r: &mut Reader, what: &str) -> Result<Vec<String>> {
    let n = r.count(what, 2)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.string(what)?);
    }
    Ok(v)
}

/// Parse the data of a skin chunk (version 0 or 1). The cursor must sit right
/// after the skin header.
pub fn parse_skin(r: &mut Reader, h: &ChunkHeader, notes: &mut Vec<String>) -> Result<Skin> {
    if h.version > 1 {
        return Err(r.err(format!("unsupported skin version {}", h.version)));
    }
    let v1 = h.version == 1;
    let mut skin = Skin { version: h.version, ..Skin::default() };

    // Version 0: three (vertex, bone, weight) lists, one per influence slot.
    // They are merged per vertex, in vertex order.
    let mut merged: BTreeMap<u32, (Vec<u32>, Vec<f32>)> = BTreeMap::new();
    if !v1 {
        for _ in 0..3 {
            let n = r.count("skin influence count", 6)?;
            for _ in 0..n {
                let vertex = r.uint("skin vertex index")?;
                let bone = r.uint("skin bone index")?;
                let weight = r.float("skin weight")?;
                let e = merged.entry(vertex).or_default();
                e.0.push(bone);
                e.1.push(weight);
            }
        }
    }

    lead_byte(r, v1, "skin objects tag")?;
    skin.skin_objects = string_array(r, "skin object count")?;
    lead_byte(r, v1, "skeleton objects tag")?;
    skin.skeleton_objects = string_array(r, "skeleton object count")?;

    if !v1 {
        let n = r.count("skin unknown vector count", 13)?;
        for _ in 0..n {
            skin.unknown_vectors.push(r.vec3("skin unknown vector")?);
        }
        skin.vertices = merged
            .into_iter()
            .map(|(vertex, (bones, weights))| SkinVertex { vertex, bones, weights })
            .collect();
    } else {
        lead_byte(r, true, "vertex indices tag")?;
        let vertex_indices = int_array(r, "vertex index count")?;
        lead_byte(r, true, "bones per vertex tag")?;
        let bone_counts = int_array(r, "bones-per-vertex count")?;
        lead_byte(r, true, "bone list tag")?;
        let bone_list = int_array(r, "bone list count")?;
        lead_byte(r, true, "weights tag")?;
        let n = r.count("weight count", 2)?;
        let mut weights = Vec::with_capacity(n);
        for _ in 0..n {
            weights.push(r.float("skin weight")?);
        }
        lead_byte(r, true, "primitive vertex counts tag")?;
        int_array(r, "primitive vertex count")?;
        lead_byte(r, true, "primitive start vertices tag")?;
        int_array(r, "primitive start count")?;

        if vertex_indices.len() != bone_counts.len() {
            return Err(r.err(format!(
                "skin has {} vertex indices but {} bone counts",
                vertex_indices.len(),
                bone_counts.len()
            )));
        }
        if bone_list.len() != weights.len() {
            notes.push(format!("skin bone list ({}) and weights ({}) differ in length", bone_list.len(), weights.len()));
        }
        // The per-vertex `vertex_indices` entry is documented only as "vertex
        // indices". Real files have been seen to hold offsets into the bone
        // list, so: if the entries equal the running sum of the bone counts,
        // treat them as offsets (vertex id = position); otherwise treat them as
        // vertex ids with offsets computed from the counts.
        let mut running = Vec::with_capacity(bone_counts.len());
        let mut acc = 0u32;
        for c in &bone_counts {
            running.push(acc);
            acc = acc.saturating_add(*c);
        }
        let as_offsets = vertex_indices == running;
        for i in 0..vertex_indices.len() {
            let offset = running[i] as usize;
            let n = bone_counts[i] as usize;
            let end = offset.saturating_add(n);
            if end > bone_list.len() || end > weights.len() {
                return Err(r.err(format!(
                    "skin vertex {i} wants bone list entries {offset}..{end} but the list has {}",
                    bone_list.len().min(weights.len())
                )));
            }
            skin.vertices.push(SkinVertex {
                vertex: if as_offsets { i as u32 } else { vertex_indices[i] },
                bones: bone_list[offset..end].to_vec(),
                weights: weights[offset..end].to_vec(),
            });
        }
    }
    Ok(skin)
}

/// One bone of a skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Index of the parent bone in [`Skeleton::bones`], `None` for roots.
    pub parent: Option<usize>,
    pub parent_name: String,
    /// Bind (rest) transform relative to the parent.
    pub bind_local: Mat4x3,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    /// Build a skeleton from mesh chunks: each mesh node is a bone. A bone
    /// whose named parent is not in the file becomes a root.
    pub fn from_meshes(meshes: &[Mesh]) -> Skeleton {
        let nodes: Vec<_> = meshes.iter().filter_map(|m| m.node.as_ref()).collect();
        let mut by_name = HashMap::new();
        for (i, n) in nodes.iter().enumerate() {
            by_name.entry(n.name.as_str()).or_insert(i);
        }
        let bones = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| Bone {
                name: n.name.clone(),
                parent: if n.has_parent { by_name.get(n.parent_name.as_str()).copied().filter(|p| *p != i) } else { None },
                parent_name: if n.has_parent { n.parent_name.clone() } else { String::new() },
                bind_local: n.transform,
            })
            .collect();
        Skeleton { bones }
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    pub fn roots(&self) -> Vec<usize> {
        (0..self.bones.len()).filter(|i| self.bones[*i].parent.is_none()).collect()
    }

    pub fn children(&self, bone: usize) -> Vec<usize> {
        (0..self.bones.len()).filter(|i| self.bones[*i].parent == Some(bone)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kfs::Node;
    use crate::testutil::*;

    fn hdr(version: u32) -> ChunkHeader {
        ChunkHeader { id: ID_SKIN, version, size: 0, data_start: 0 }
    }

    #[test]
    fn skin_v0_merges_slots() {
        let mut p = Vec::new();
        // slot 1: vertex 0 -> bone 0 (0.75), vertex 1 -> bone 1 (1.0)
        p.extend(int(2));
        for (v, b, w) in [(0, 0, 0.75), (1, 1, 1.0)] {
            p.extend(int(v));
            p.extend(int(b));
            p.extend(float(w));
        }
        // slot 2: vertex 0 -> bone 1 (0.25)
        p.extend(int(1));
        p.extend(int(0));
        p.extend(int(1));
        p.extend(float(0.25));
        p.extend(int(0)); // slot 3 empty
        p.extend(int(1));
        p.extend(string("body"));
        p.extend(int(2));
        p.extend(string("Bip01"));
        p.extend(string("Bip01 Spine"));
        p.extend(int(1));
        p.extend(vec3([1., 2., 3.]));
        let s = parse_skin(&mut Reader::new(&p), &hdr(0), &mut Vec::new()).unwrap();
        assert_eq!(s.skeleton_objects, vec!["Bip01", "Bip01 Spine"]);
        assert_eq!(s.vertices.len(), 2);
        assert_eq!(s.vertices[0].bones, vec![0, 1]);
        assert_eq!(s.vertices[0].weights, vec![0.75, 0.25]);
        assert_eq!(s.unknown_vectors, vec![[1., 2., 3.]]);
    }

    fn arr_i(tag: u8, v: &[u32]) -> Vec<u8> {
        let mut b = vec![tag];
        b.extend(int(v.len() as i32));
        for x in v {
            b.extend(int(*x as i32));
        }
        b
    }

    #[test]
    fn skin_v1_offsets() {
        let mut data = vec![0u8];
        data.extend(int(1));
        data.extend(string("body"));
        data.push(0);
        data.extend(int(2));
        data.extend(string("A"));
        data.extend(string("B"));
        // vertex 0 uses bones [0,1], vertex 1 uses bone [1]
        data.extend(arr_i(0, &[0, 2]));
        data.extend(arr_i(0, &[2, 1]));
        data.extend(arr_i(0, &[0, 1, 1]));
        data.push(0);
        data.extend(int(3));
        for w in [0.5, 0.5, 1.0] {
            data.extend(float(w));
        }
        data.extend(arr_i(0, &[2]));
        data.extend(arr_i(0, &[0]));
        let s = parse_skin(&mut Reader::new(&data), &hdr(1), &mut Vec::new()).unwrap();
        assert_eq!(s.skin_objects, vec!["body"]);
        assert_eq!(s.vertices.len(), 2);
        assert_eq!(s.vertices[0], SkinVertex { vertex: 0, bones: vec![0, 1], weights: vec![0.5, 0.5] });
        assert_eq!(s.vertices[1], SkinVertex { vertex: 1, bones: vec![1], weights: vec![1.0] });
    }

    #[test]
    fn skin_truncated_is_error() {
        let mut p = int(5);
        p.extend(int(0));
        assert!(parse_skin(&mut Reader::new(&p), &hdr(0), &mut Vec::new()).is_err());
    }

    fn bone_mesh(name: &str, parent: &str, has_parent: bool) -> Mesh {
        Mesh {
            node: Some(Node {
                name: name.into(),
                parent_name: parent.into(),
                has_parent,
                transform: crate::xform::IDENTITY,
                user_string: String::new(),
            }),
            ..Mesh::default()
        }
    }

    #[test]
    fn skeleton_hierarchy() {
        let sk = Skeleton::from_meshes(&[
            bone_mesh("root", "", false),
            bone_mesh("spine", "root", true),
            bone_mesh("head", "spine", true),
            bone_mesh("lost", "missing", true),
        ]);
        assert_eq!(sk.roots(), vec![0, 3]);
        assert_eq!(sk.children(0), vec![1]);
        assert_eq!(sk.bones[2].parent, Some(1));
        assert_eq!(sk.index_of("head"), Some(2));
    }
}
