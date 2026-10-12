//! Mesh and material chunks (the content of `.kfs` files, and the mesh and
//! material-list chunks inside `.kf2` files).
//!
//! In Max Payne a mesh chunk doubles as a skeleton node: every bone of a
//! character rig is one (usually geometry-less) mesh whose node names its
//! parent. See `docs/formats/kfs.md`.

use crate::reader::{ChunkHeader, ParseError, Reader, Result};
use crate::xform::Mat4x3;

pub const ID_MATERIAL_LIST: u32 = 0x0001_000F;
pub const ID_MESH: u32 = 0x0001_0005;
pub const ID_NODE: u32 = 0x0001_0000;
pub const ID_GEOMETRY: u32 = 0x0001_0006;
pub const ID_POLYGONS: u32 = 0x0001_0007;
pub const ID_POLYGON: u32 = 0x0001_0008;
pub const ID_SMOOTHING: u32 = 0x0001_000B;
pub const ID_POLYGON_MATERIAL: u32 = 0x0001_000C;
pub const ID_UV_MAPPING: u32 = 0x0001_000E;
pub const ID_MATERIAL: u32 = 0x0001_0010;
pub const ID_TEXTURE: u32 = 0x0001_0011;
pub const ID_REFERENCE: u32 = 0x0001_001A;

/// Scene node: a name, its parent and the transform to the parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent_name: String,
    pub has_parent: bool,
    /// Bind (rest) transform relative to the parent.
    pub transform: Mat4x3,
    pub user_string: String,
}

/// One texture-coordinate set.
#[derive(Debug, Clone, PartialEq)]
pub struct UvLayer {
    pub layer: u32,
    /// `u, v, w` triples.
    pub coords: Vec<[f32; 3]>,
    /// Per-triangle indices into `coords`, parallel to `Mesh::triangles`.
    /// `None` means the coordinates are indexed like the positions.
    pub triangles: Option<Vec<[u32; 3]>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub version: u32,
    pub node: Option<Node>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Triangles as indices into `positions`, already offset per primitive.
    pub triangles: Vec<[u32; 3]>,
    /// Index into `material_names` for each triangle (primitive index in the
    /// version 2 layout).
    pub triangle_material: Vec<u32>,
    pub material_names: Vec<String>,
    pub uv_layers: Vec<UvLayer>,
    pub smoothing_groups: Vec<u32>,
    /// Name of the object this mesh references instead of owning geometry.
    pub reference: Option<String>,
}

impl Mesh {
    pub fn name(&self) -> &str {
        self.node.as_ref().map_or("", |n| n.name.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextureAnim {
    pub automatic_start: bool,
    pub random_start_frame: bool,
    pub start_frame: i64,
    pub fps: i64,
    /// 0 loop, 1 ping-pong, 2 hold.
    pub end_condition: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub name: String,
    pub mip_maps: i64,
    pub filtering: i64,
    pub files: Vec<String>,
    pub anim: Option<TextureAnim>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Material {
    pub version: u32,
    pub name: String,
    pub two_sided: bool,
    pub fogging: bool,
    pub diffuse_combined: bool,
    pub invisible_geometry: bool,
    pub has_vertex_alpha: bool,
    pub diffuse_color_type: i64,
    pub specular_color_type: i64,
    pub lit_type: i64,
    pub ambient: [f32; 4],
    pub diffuse: [f32; 4],
    pub specular: [f32; 4],
    pub vertex_alpha: f32,
    pub specular_exponent: f32,
    pub diffuse_texture_type: i64,
    pub reflection_texture_type: i64,
    pub emboss_factor: f32,
    pub diffuse_texture: Option<Texture>,
    pub reflection_texture: Option<Texture>,
    pub bump_texture: Option<Texture>,
    pub opacity_texture: Option<Texture>,
    pub mask_texture: Option<Texture>,
    pub mask_texture_type: i64,
    pub has_lit: bool,
    /// Version 2 fields of unknown meaning, kept for inspection.
    pub unknown_v2: Option<(bool, bool, i64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialList {
    /// Semicolon-separated texture directories.
    pub texture_dirs: String,
    pub materials: Vec<Material>,
}

fn expect_id(h: &ChunkHeader, want: u32, what: &str, r: &Reader) -> Result<()> {
    if h.id != want {
        return Err(ParseError {
            offset: h.data_start.saturating_sub(13).min(r.len()),
            msg: format!("expected {what} chunk (id 0x{want:08x}), found id 0x{:08x}", h.id),
        });
    }
    Ok(())
}

fn bad_version(r: &Reader, what: &str, v: u32) -> ParseError {
    r.err(format!("unsupported {what} version {v}"))
}

/// Read a sub-chunk header and check its id.
fn sub(r: &mut Reader, want: u32, what: &str) -> Result<ChunkHeader> {
    let h = r.chunk_header(what)?;
    expect_id(&h, want, what, r)?;
    Ok(h)
}

pub fn parse_node(r: &mut Reader, h: &ChunkHeader) -> Result<Node> {
    if h.version > 1 {
        return Err(bad_version(r, "node", h.version));
    }
    let name = r.string("node name")?;
    let parent_name = r.string("node parent name")?;
    let transform = r.matrix4x3("node object-to-parent transform")?;
    let has_parent = r.boolean("node has_parent")?;
    let user_string = if h.version > 0 { r.string("node user string")? } else { String::new() };
    Ok(Node { name, parent_name, has_parent, transform, user_string })
}

struct Geometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    /// Vertices per primitive.
    primitives: Vec<u32>,
}

fn parse_geometry(r: &mut Reader, h: &ChunkHeader) -> Result<Geometry> {
    match h.version {
        0 => {
            let n = r.count("geometry vertex count", 13)?;
            let mut positions = Vec::with_capacity(n);
            for _ in 0..n {
                positions.push(r.vec3("geometry vertex")?);
            }
            Ok(Geometry { primitives: vec![n as u32], positions, normals: Vec::new() })
        }
        1 => {
            let n = r.count("geometry vertex count", 24)?;
            let mut positions = Vec::with_capacity(n);
            for _ in 0..n {
                positions.push(r.vec3_raw("geometry vertex")?);
            }
            let mut normals = Vec::with_capacity(n);
            for _ in 0..n {
                normals.push(r.vec3_raw("geometry normal")?);
            }
            let np = r.count("geometry primitive count", 2)?;
            let mut primitives = Vec::with_capacity(np);
            for _ in 0..np {
                primitives.push(r.uint("vertices per primitive")?);
            }
            Ok(Geometry { positions, normals, primitives })
        }
        v => Err(bad_version(r, "geometry", v)),
    }
}

/// Fan-triangulate a polygon's vertex list.
fn fan(indices: &[u32], out: &mut Vec<[u32; 3]>) {
    for i in 2..indices.len() {
        out.push([indices[0], indices[i - 1], indices[i]]);
    }
}

struct Polygons {
    triangles: Vec<[u32; 3]>,
    /// Triangles per primitive.
    primitives: Vec<u32>,
    /// Source polygon of each triangle (identity for the version 1 layout).
    source_polygon: Vec<u32>,
    notes: Vec<String>,
}

fn parse_polygon(r: &mut Reader) -> Result<Vec<u32>> {
    let h = sub(r, ID_POLYGON, "polygon")?;
    if h.version != 0 {
        return Err(bad_version(r, "polygon", h.version));
    }
    let n = r.count("polygon vertex count", 2)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.uint("polygon vertex index")?);
    }
    Ok(v)
}

fn parse_polygons(r: &mut Reader, h: &ChunkHeader) -> Result<Polygons> {
    match h.version {
        0 => {
            let n = r.count("polygon count", 13)?;
            let mut triangles = Vec::new();
            let mut source_polygon = Vec::new();
            for p in 0..n {
                let idx = parse_polygon(r)?;
                let before = triangles.len();
                fan(&idx, &mut triangles);
                source_polygon.extend(std::iter::repeat_n(p as u32, triangles.len() - before));
            }
            r.int("polygons unknown1")?;
            r.int("polygons unknown2")?;
            Ok(Polygons {
                primitives: vec![triangles.len() as u32],
                triangles,
                source_polygon,
                notes: Vec::new(),
            })
        }
        1 => {
            // The count is the number of u16 indices, three per triangle.
            let n = r.count("polygon index count", 2)?;
            let mut flat = Vec::with_capacity(n);
            for _ in 0..n {
                flat.push(r.u16_raw("polygon index")? as u32);
            }
            let mut notes = Vec::new();
            if flat.len() % 3 != 0 {
                notes.push(format!("polygon index count {} is not a multiple of 3, dropping the tail", flat.len()));
            }
            let triangles: Vec<[u32; 3]> = flat.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2]]).collect();
            let np = r.count("polygon primitive count", 2)?;
            let mut primitives = Vec::with_capacity(np);
            for _ in 0..np {
                primitives.push(r.uint("triangles per primitive")?);
            }
            let source_polygon = (0..triangles.len() as u32).collect();
            Ok(Polygons { triangles, primitives, source_polygon, notes })
        }
        v => Err(bad_version(r, "polygons", v)),
    }
}

struct PolygonMaterial {
    names: Vec<String>,
    /// Per source polygon, version 0 only.
    per_polygon: Option<Vec<u32>>,
}

fn parse_polygon_material(r: &mut Reader, h: &ChunkHeader) -> Result<PolygonMaterial> {
    if h.version > 1 {
        return Err(bad_version(r, "polygon material", h.version));
    }
    let n = r.count("material count", 2)?;
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        names.push(r.string("material name")?);
    }
    let per_polygon = if h.version == 0 {
        let np = r.count("material polygon count", 2)?;
        let mut v = Vec::with_capacity(np);
        for _ in 0..np {
            v.push(r.uint("material index for polygon")?);
        }
        Some(v)
    } else {
        None
    };
    Ok(PolygonMaterial { names, per_polygon })
}

/// UV chunk, plus (version 0 only) the per-polygon index lists, still
/// un-triangulated.
struct RawUv {
    layer: u32,
    coords: Vec<[f32; 3]>,
    polygon_indices: Option<Vec<Vec<u32>>>,
}

fn parse_uv(r: &mut Reader, h: &ChunkHeader) -> Result<RawUv> {
    match h.version {
        0 => {
            let layer = r.uint("uv layer")?;
            let n = r.count("uv polygon count", 13)?;
            let mut polys = Vec::with_capacity(n);
            for _ in 0..n {
                polys.push(parse_polygon(r)?);
            }
            r.int("uv reserved1")?;
            r.int("uv reserved2")?;
            let nc = r.count("uv coordinate count", 13)?;
            let mut coords = Vec::with_capacity(nc);
            for _ in 0..nc {
                coords.push(r.vec3("uv coordinate")?);
            }
            Ok(RawUv { layer, coords, polygon_indices: Some(polys) })
        }
        1 => {
            let layer = r.uint("uv layer")?;
            let nc = r.count("uv coordinate count", 12)?;
            let mut coords = Vec::with_capacity(nc);
            for _ in 0..nc {
                coords.push(r.vec3_raw("uv coordinate")?);
            }
            let np = r.count("uv primitive count", 2)?;
            for _ in 0..np {
                r.uint("uv coordinates per primitive")?;
            }
            Ok(RawUv { layer, coords, polygon_indices: None })
        }
        v => Err(bad_version(r, "uv mapping", v)),
    }
}

fn parse_smoothing(r: &mut Reader, h: &ChunkHeader) -> Result<Vec<u32>> {
    if h.version != 0 {
        return Err(bad_version(r, "smoothing", h.version));
    }
    let n = r.count("smoothing group count", 2)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.uint("smoothing group")?);
    }
    Ok(v)
}

fn is_mesh_child(id: u32) -> bool {
    matches!(
        id,
        ID_NODE | ID_GEOMETRY | ID_POLYGONS | ID_SMOOTHING | ID_POLYGON_MATERIAL | ID_UV_MAPPING | ID_REFERENCE
    )
}

/// Warn when a chunk's parsed length disagrees with its declared size.
pub fn check_size(r: &Reader, h: &ChunkHeader, label: &str, notes: &mut Vec<String>) {
    if r.pos() != h.data_end() {
        notes.push(format!(
            "{label} chunk at 0x{:x}: parsed {} bytes but header says {}",
            h.data_start.saturating_sub(13),
            r.pos().saturating_sub(h.data_start.saturating_sub(13)),
            h.size
        ));
    }
}

/// Parse the data of a mesh chunk (version 1 or 2). The cursor must sit right
/// after the mesh header. Sub-chunks may come in any order and any of them may
/// be missing; the loop stops at the first chunk that is not a mesh child.
pub fn parse_mesh(r: &mut Reader, h: &ChunkHeader, notes: &mut Vec<String>) -> Result<Mesh> {
    if h.version != 1 && h.version != 2 {
        return Err(bad_version(r, "mesh", h.version));
    }
    let mut mesh = Mesh { version: h.version, ..Mesh::default() };
    let mut geometry = None;
    let mut polygons = None;
    let mut pmat = None;
    let mut uvs = Vec::new();

    while let Some(id) = r.peek_chunk_id() {
        if !is_mesh_child(id) {
            break;
        }
        let ch = r.chunk_header("mesh sub-chunk")?;
        let label = match id {
            ID_NODE => {
                mesh.node = Some(parse_node(r, &ch)?);
                "node"
            }
            ID_GEOMETRY => {
                geometry = Some(parse_geometry(r, &ch)?);
                "geometry"
            }
            ID_POLYGONS => {
                polygons = Some(parse_polygons(r, &ch)?);
                "polygons"
            }
            ID_POLYGON_MATERIAL => {
                pmat = Some(parse_polygon_material(r, &ch)?);
                "polygon material"
            }
            ID_UV_MAPPING => {
                uvs.push(parse_uv(r, &ch)?);
                "uv mapping"
            }
            ID_SMOOTHING => {
                mesh.smoothing_groups = parse_smoothing(r, &ch)?;
                "smoothing"
            }
            _ => {
                if ch.version != 0 {
                    return Err(bad_version(r, "reference to data", ch.version));
                }
                mesh.reference = Some(r.string("referenced object name")?);
                "reference to data"
            }
        };
        check_size(r, &ch, label, notes);
    }

    if let Some(g) = geometry {
        mesh.positions = g.positions;
        mesh.normals = g.normals;
        if let Some(p) = polygons {
            assemble_triangles(&mut mesh, &g.primitives, p, pmat.as_ref(), notes)?;
            for u in uvs {
                mesh.uv_layers.push(assemble_uv(&mesh, u, &mut *notes));
            }
        } else if !uvs.is_empty() {
            notes.push(format!("mesh '{}' has UVs but no polygons, dropping them", mesh.name()));
        }
    } else if polygons.is_some() {
        notes.push(format!("mesh '{}' has polygons but no geometry, dropping them", mesh.name()));
    }
    if let Some(pm) = pmat {
        mesh.material_names = pm.names;
    }
    Ok(mesh)
}

fn assemble_triangles(
    mesh: &mut Mesh,
    vertex_primitives: &[u32],
    p: Polygons,
    pmat: Option<&PolygonMaterial>,
    notes: &mut Vec<String>,
) -> Result<()> {
    notes.extend(p.notes);
    // Vertex start of each primitive: the version 2 layout indexes per primitive.
    let mut starts = Vec::with_capacity(vertex_primitives.len());
    let mut acc = 0u32;
    for n in vertex_primitives {
        starts.push(acc);
        acc = acc.saturating_add(*n);
    }
    let mut t = 0usize;
    for (prim, count) in p.primitives.iter().enumerate() {
        let start = starts.get(prim).copied().unwrap_or(0);
        for _ in 0..*count {
            let Some(tri) = p.triangles.get(t) else { break };
            let mut out = [0u32; 3];
            for (o, i) in out.iter_mut().zip(tri) {
                *o = i.saturating_add(start);
                if *o as usize >= mesh.positions.len() {
                    return Err(ParseError {
                        offset: 0,
                        msg: format!(
                            "mesh '{}': triangle {t} uses vertex {} but there are only {} vertices",
                            mesh.name(),
                            *o,
                            mesh.positions.len()
                        ),
                    });
                }
            }
            mesh.triangles.push(out);
            let mat = match pmat.and_then(|m| m.per_polygon.as_ref()) {
                Some(per_polygon) => per_polygon.get(p.source_polygon[t] as usize).copied().unwrap_or(0),
                None => prim as u32,
            };
            mesh.triangle_material.push(mat);
            t += 1;
        }
    }
    if t < p.triangles.len() {
        notes.push(format!(
            "mesh '{}': {} triangles not covered by any primitive were dropped",
            mesh.name(),
            p.triangles.len() - t
        ));
    }
    Ok(())
}

fn assemble_uv(mesh: &Mesh, u: RawUv, notes: &mut Vec<String>) -> UvLayer {
    let triangles = u.polygon_indices.and_then(|polys| {
        let mut tris = Vec::new();
        for p in &polys {
            fan(p, &mut tris);
        }
        if tris.len() == mesh.triangles.len() {
            Some(tris)
        } else {
            notes.push(format!(
                "mesh '{}': UV layer {} has {} triangles but the mesh has {}, ignoring UV indices",
                mesh.name(),
                u.layer,
                tris.len(),
                mesh.triangles.len()
            ));
            None
        }
    });
    UvLayer { layer: u.layer, coords: u.coords, triangles }
}

fn parse_texture(r: &mut Reader) -> Result<Texture> {
    let h = sub(r, ID_TEXTURE, "texture")?;
    if h.version != 1 {
        return Err(bad_version(r, "texture", h.version));
    }
    let name = r.string("texture name")?;
    let mip_maps = r.int("texture mip map count")?;
    let filtering = r.int("texture filtering")?;
    let n = r.count("texture file count", 2)?;
    let mut files = Vec::with_capacity(n);
    for _ in 0..n {
        files.push(r.string("texture file name")?);
    }
    // The spec says "more than zero"; the reference implementation reads the
    // animation block only for more than one file. A single-frame texture has
    // nothing to animate, so we follow the latter.
    let anim = if n > 1 {
        Some(TextureAnim {
            automatic_start: r.boolean("texture automatic start")?,
            random_start_frame: r.boolean("texture random start frame")?,
            start_frame: r.int("texture start frame")?,
            fps: r.int("texture playback fps")?,
            end_condition: r.int("texture end condition")?,
        })
    } else {
        None
    };
    Ok(Texture { name, mip_maps, filtering, files, anim })
}

fn opt_texture(r: &mut Reader, what: &str) -> Result<Option<Texture>> {
    if r.boolean(what)? { parse_texture(r).map(Some) } else { Ok(None) }
}

fn parse_material(r: &mut Reader) -> Result<Material> {
    let h = sub(r, ID_MATERIAL, "material")?;
    if h.version > 2 {
        return Err(bad_version(r, "material", h.version));
    }
    let mut m = Material { version: h.version, ..Material::default() };
    m.name = r.string("material name")?;
    m.two_sided = r.boolean("material two sided")?;
    m.fogging = r.boolean("material fogging")?;
    m.diffuse_combined = r.boolean("material diffuse combined")?;
    m.invisible_geometry = r.boolean("material invisible geometry")?;
    m.has_vertex_alpha = r.boolean("material has vertex alpha")?;
    m.diffuse_color_type = r.int("material diffuse color type")?;
    m.specular_color_type = r.int("material specular color type")?;
    m.lit_type = r.int("material lit type")?;
    for c in [&mut m.ambient, &mut m.diffuse, &mut m.specular] {
        for v in c.iter_mut() {
            *v = r.float("material color component")?;
        }
    }
    m.vertex_alpha = r.float("material vertex alpha")?;
    m.specular_exponent = r.float("material specular exponent")?;
    m.diffuse_texture_type = r.int("material diffuse texture type")?;
    m.reflection_texture_type = r.int("material reflection texture type")?;
    m.emboss_factor = r.float("material emboss factor")?;
    m.diffuse_texture = opt_texture(r, "material has diffuse texture")?;
    m.reflection_texture = opt_texture(r, "material has reflection texture")?;
    m.bump_texture = opt_texture(r, "material has bump texture")?;
    m.opacity_texture = opt_texture(r, "material has opacity texture")?;
    if h.version > 0 {
        m.mask_texture = opt_texture(r, "material has mask texture")?;
        m.mask_texture_type = r.int("material mask texture type")?;
        m.has_lit = r.boolean("material has lit")?;
    }
    if h.version > 1 {
        m.unknown_v2 = Some((r.boolean("material unknown1")?, r.boolean("material unknown2")?, r.int("material unknown3")?));
    }
    Ok(m)
}

/// Parse the data of a material list chunk (version 0).
pub fn parse_material_list(r: &mut Reader, h: &ChunkHeader) -> Result<MaterialList> {
    if h.version != 0 {
        return Err(bad_version(r, "material list", h.version));
    }
    let texture_dirs = r.string("texture directories")?;
    let n = r.count("material count", 13)?;
    let mut materials = Vec::with_capacity(n);
    for _ in 0..n {
        materials.push(parse_material(r)?);
    }
    Ok(MaterialList { texture_dirs, materials })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    pub fn node_chunk(name: &str, parent: &str, m: [[f32; 3]; 4], has_parent: bool) -> Vec<u8> {
        let mut p = string(name);
        p.extend(string(parent));
        p.extend(matrix(m));
        p.extend(boolean(has_parent));
        p.extend(string("user"));
        chunk(ID_NODE, 1, &p)
    }

    #[test]
    fn mesh_v2_two_primitives() {
        let mut geo = uint(4);
        for v in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [5., 5., 5.]] {
            geo.extend(raw_vec3(v));
        }
        for _ in 0..4 {
            geo.extend(raw_vec3([0., 0., 1.]));
        }
        geo.extend(uint(2));
        geo.extend(uint(3));
        geo.extend(uint(1));
        // Primitive 0 has 3 vertices, primitive 1 has 1 (so triangle (0,0,0) there maps to vertex 3).
        let mut polys = uint(6);
        for i in [0u16, 1, 2, 0, 0, 0] {
            polys.extend(i.to_le_bytes());
        }
        polys.extend(uint(2));
        polys.extend(uint(1));
        polys.extend(uint(1));
        let mut pm = int(2);
        pm.extend(string("skin"));
        pm.extend(string("jacket"));
        let mut uv = uint(0);
        uv.extend(uint(1));
        uv.extend(raw_vec3([0.5, 0.25, 0.0]));
        uv.extend(uint(1));
        uv.extend(uint(1));

        let mut file = Vec::new();
        file.extend(node_chunk("Bip01 Head", "Bip01 Neck", IDENT, true));
        file.extend(chunk(ID_GEOMETRY, 1, &geo));
        file.extend(chunk(ID_POLYGONS, 1, &polys));
        file.extend(chunk(ID_POLYGON_MATERIAL, 1, &pm));
        file.extend(chunk(ID_UV_MAPPING, 1, &uv));
        let tail = chunk(ID_MATERIAL_LIST, 0, &[]); // a following non-child chunk ends the mesh
        file.extend(&tail);

        let mut r = Reader::new(&file);
        let h = ChunkHeader { id: ID_MESH, version: 2, size: 0, data_start: 0 };
        let mut notes = Vec::new();
        let m = parse_mesh(&mut r, &h, &mut notes).unwrap();
        assert_eq!(m.name(), "Bip01 Head");
        assert_eq!(m.node.as_ref().unwrap().parent_name, "Bip01 Neck");
        assert_eq!(m.positions.len(), 4);
        assert_eq!(m.triangles, vec![[0, 1, 2], [3, 3, 3]]);
        assert_eq!(m.triangle_material, vec![0, 1]);
        assert_eq!(m.material_names, vec!["skin", "jacket"]);
        assert_eq!(m.uv_layers[0].coords, vec![[0.5, 0.25, 0.0]]);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(r.peek_chunk_id(), Some(ID_MATERIAL_LIST));
    }

    #[test]
    fn mesh_v1_polygon_chunks_fan() {
        let mut geo = int(4);
        for v in [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]] {
            geo.extend(vec3(v));
        }
        let mut quad = int(4);
        for i in 0..4 {
            quad.extend(int(i));
        }
        let mut polys = int(1);
        polys.extend(chunk(ID_POLYGON, 0, &quad));
        polys.extend(int(0));
        polys.extend(int(0));
        let mut pm = int(1);
        pm.extend(string("m0"));
        pm.extend(int(1));
        pm.extend(int(0));
        let mut file = Vec::new();
        file.extend(chunk(ID_GEOMETRY, 0, &geo));
        file.extend(chunk(ID_POLYGONS, 0, &polys));
        file.extend(chunk(ID_POLYGON_MATERIAL, 0, &pm));
        let mut r = Reader::new(&file);
        let h = ChunkHeader { id: ID_MESH, version: 1, size: 0, data_start: 0 };
        let m = parse_mesh(&mut r, &h, &mut Vec::new()).unwrap();
        assert_eq!(m.triangles, vec![[0, 1, 2], [0, 2, 3]]);
        assert_eq!(m.triangle_material, vec![0, 0]);
    }

    #[test]
    fn out_of_range_index_is_error() {
        let mut geo = uint(1);
        geo.extend(raw_vec3([0.; 3]));
        geo.extend(raw_vec3([0.; 3]));
        geo.extend(uint(1));
        geo.extend(uint(1));
        let mut polys = uint(3);
        for i in [0u16, 1, 2] {
            polys.extend(i.to_le_bytes());
        }
        polys.extend(uint(1));
        polys.extend(uint(1));
        let mut file = chunk(ID_GEOMETRY, 1, &geo);
        file.extend(chunk(ID_POLYGONS, 1, &polys));
        let mut r = Reader::new(&file);
        let h = ChunkHeader { id: ID_MESH, version: 2, size: 0, data_start: 0 };
        assert!(parse_mesh(&mut r, &h, &mut Vec::new()).is_err());
    }

    #[test]
    fn material_list_with_texture() {
        let mut tex = string("diff");
        tex.extend(int(0));
        tex.extend(int(2));
        tex.extend(int(1));
        tex.extend(string("diff.ras"));
        let mut mat = string("skin");
        for b in [false, true, false, false, false] {
            mat.extend(boolean(b));
        }
        for v in [0, 0, 0] {
            mat.extend(int(v));
        }
        for _ in 0..12 {
            mat.extend(float(0.5));
        }
        mat.extend(float(1.0));
        mat.extend(float(8.0));
        mat.extend(int(0));
        mat.extend(int(0));
        mat.extend(float(0.0));
        mat.extend(boolean(true));
        mat.extend(chunk(ID_TEXTURE, 1, &tex));
        for _ in 0..3 {
            mat.extend(boolean(false));
        }
        let mut list = string("tex;tex2");
        list.extend(int(1));
        list.extend(chunk(ID_MATERIAL, 0, &mat));
        let mut r = Reader::new(&list);
        let h = ChunkHeader { id: ID_MATERIAL_LIST, version: 0, size: 0, data_start: 0 };
        let ml = parse_material_list(&mut r, &h).unwrap();
        assert_eq!(ml.texture_dirs, "tex;tex2");
        let m = &ml.materials[0];
        assert!(m.fogging && !m.two_sided);
        assert_eq!(m.diffuse_texture.as_ref().unwrap().files, vec!["diff.ras"]);
        assert_eq!(m.specular_exponent, 8.0);
        assert!(r.is_empty());
    }
}
