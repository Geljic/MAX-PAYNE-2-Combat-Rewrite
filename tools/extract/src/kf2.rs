//! The `.kf2` container: a flat list of chunks (meshes, skins, keyframe
//! animations, cameras...). `.kfs` and `.skd` files use the same container, so
//! [`parse`] reads all three. See `docs/formats/kf2.md`.

use crate::kfs::{self, Mesh, MaterialList, Node};
use crate::reader::{CHUNK_HEADER_LEN, ChunkHeader, ParseError, Reader, Result};
use crate::skd::{self, Skeleton, Skin};
use crate::xform::Mat4x3;

pub const ID_CAMERA: u32 = 0x0001_0001;
pub const ID_POINT_LIGHT: u32 = 0x0001_0002;
pub const ID_DIRECTIONAL_LIGHT: u32 = 0x0001_0003;
pub const ID_SPOT_LIGHT: u32 = 0x0001_0004;
pub const ID_KEYFRAME_ANIMATION: u32 = 0x0001_0012;
pub const ID_ANIMATION: u32 = 0x0001_0013;
pub const ID_ENVIRONMENT: u32 = 0x0001_0015;
pub const ID_HELPER: u32 = 0x0001_0016;
pub const ID_POINT_LIGHT_ANIMATION: u32 = 0x0001_0017;
pub const ID_DIRECTIONAL_LIGHT_ANIMATION: u32 = 0x0001_0018;
pub const ID_SPOT_LIGHT_ANIMATION: u32 = 0x0001_0019;

/// One keyframe: a frame number and the node's transform relative to its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub frame: u32,
    pub transform: Mat4x3,
}

/// Animation of a single node (bone): the contents of one keyframe animation chunk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub version: u32,
    /// Name of the animated node (the sub-chunk's object name).
    pub object_name: String,
    pub fps: u32,
    pub looping: bool,
    pub parent_name: String,
    pub use_loop_interpolation: bool,
    /// Length of the animation in frames, after applying the version rule
    /// (versions below 5 store one less than the real count).
    pub total_frames: u32,
    pub keys: Vec<Key>,
    /// `(frame, visibility)` pairs, version 1 and up.
    pub visibility: Vec<(u32, f32)>,
    /// Frame the loop jumps back to, version 2 and up.
    pub loop_to_frame: u32,
    /// Frame-to-frame interpolation method id, version 3 and up. Meaning unknown.
    pub interpolation_method: i64,
    pub maintain_matrix_scaling: bool,
}

impl Track {
    /// Last frame index this track covers (either the declared length or the
    /// last key, whichever is later).
    pub fn last_frame(&self) -> u32 {
        let declared = self.total_frames.saturating_sub(1);
        self.keys.iter().map(|k| k.frame).fold(declared, u32::max)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub node: Node,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Environment {
    /// RGBA.
    pub ambient: [u8; 4],
    /// RGBA.
    pub clear: [u8; 4],
}

/// Everything read from a `.kf2`, `.kfs` or `.skd` file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Kf2File {
    pub material_lists: Vec<MaterialList>,
    pub meshes: Vec<Mesh>,
    pub skins: Vec<Skin>,
    pub tracks: Vec<Track>,
    pub cameras: Vec<Camera>,
    pub environments: Vec<Environment>,
    /// Things that were skipped or looked off, for the caller to log.
    pub notes: Vec<String>,
}

/// All tracks of a file seen as one animation clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip<'a> {
    pub name: String,
    pub fps: u32,
    /// Length in frames (longest track).
    pub frame_count: u32,
    pub duration_secs: f32,
    pub looping: bool,
    pub tracks: Vec<&'a Track>,
}

impl Kf2File {
    /// Skeleton from the mesh nodes of this file.
    pub fn skeleton(&self) -> Skeleton {
        Skeleton::from_meshes(&self.meshes)
    }

    /// The file's keyframe tracks as a clip named `name` (a file holds one
    /// clip; its name is the file name). `None` if there are no tracks.
    ///
    /// Duration is the last frame index divided by the frame rate, so a
    /// single-frame pose lasts zero seconds.
    pub fn clip(&self, name: &str) -> Option<Clip<'_>> {
        let first = self.tracks.first()?;
        let fps = first.fps;
        let last = self.tracks.iter().map(Track::last_frame).max().unwrap_or(0);
        Some(Clip {
            name: name.to_string(),
            fps,
            frame_count: last.saturating_add(1),
            duration_secs: if fps > 0 { last as f32 / fps as f32 } else { 0.0 },
            looping: self.tracks.iter().any(|t| t.looping),
            tracks: self.tracks.iter().collect(),
        })
    }
}

fn parse_track(r: &mut Reader, h: &ChunkHeader, notes: &mut Vec<String>) -> Result<Track> {
    if h.version > 5 {
        return Err(r.err(format!("unsupported keyframe animation version {}", h.version)));
    }
    let ah = r.chunk_header("animation sub-chunk")?;
    if ah.id != ID_ANIMATION {
        return Err(r.err(format!("expected animation sub-chunk (0x{ID_ANIMATION:08x}), found 0x{:08x}", ah.id)));
    }
    if ah.version != 0 {
        return Err(r.err(format!("unsupported animation sub-chunk version {}", ah.version)));
    }
    let mut t = Track { version: h.version, ..Track::default() };
    t.object_name = r.string("animation object name")?;
    t.fps = r.uint("animation fps")?;
    t.looping = r.boolean("animation is_looping")?;
    kfs::check_size(r, &ah, "animation", notes);

    t.parent_name = r.string("animation parent name")?;
    t.use_loop_interpolation = r.boolean("use loop interpolation")?;
    let total = r.uint("total keyframes")?;
    let n = r.count("keyframe count", 50)?;
    t.keys.reserve(n);
    for _ in 0..n {
        let frame = r.uint("keyframe frame id")?;
        let transform = r.matrix4x3("keyframe transform")?;
        t.keys.push(Key { frame, transform });
    }
    if h.version >= 1 {
        let n = r.count("visibility frame count", 7)?;
        for _ in 0..n {
            t.visibility.push((r.uint("visibility frame id")?, r.float("visibility")?));
        }
    }
    if h.version >= 2 {
        t.loop_to_frame = r.uint("loop to frame")?;
    }
    if h.version >= 3 {
        t.interpolation_method = r.int("frame interpolation method")?;
    }
    if h.version >= 4 {
        t.maintain_matrix_scaling = r.boolean("maintain matrix scaling")?;
    }
    t.total_frames = if h.version < 5 { total.saturating_add(1) } else { total };
    Ok(t)
}

fn parse_camera(r: &mut Reader, notes: &mut Vec<String>) -> Result<Camera> {
    let nh = r.chunk_header("camera node")?;
    if nh.id != kfs::ID_NODE {
        return Err(r.err(format!("expected node sub-chunk in camera, found 0x{:08x}", nh.id)));
    }
    let node = kfs::parse_node(r, &nh)?;
    kfs::check_size(r, &nh, "camera node", notes);
    Ok(Camera {
        node,
        fov: r.float("camera fov")?,
        near: r.float("camera front plane")?,
        far: r.float("camera back plane")?,
    })
}

fn parse_environment(r: &mut Reader) -> Result<Environment> {
    let mut rgba = |what: &str| -> Result<[u8; 4]> {
        let mut c = [0u8; 4];
        for v in &mut c {
            *v = r.int(what)? as u8;
        }
        Ok(c)
    };
    Ok(Environment { ambient: rgba("ambient light color")?, clear: rgba("clearing color")? })
}

/// Parse a `.kf2`, `.kfs` or `.skd` file. Unknown chunk types are skipped
/// using their declared size and noted in [`Kf2File::notes`]; malformed data
/// is an error, never a panic.
pub fn parse(data: &[u8]) -> Result<Kf2File> {
    let mut r = Reader::new(data);
    let mut f = Kf2File::default();
    while !r.is_empty() {
        let start = r.pos();
        if r.remaining() < CHUNK_HEADER_LEN {
            return Err(r.err(format!("{} stray bytes at end of file, not enough for a chunk header", r.remaining())));
        }
        let h = r.chunk_header("top-level chunk")?;
        let mut notes = Vec::new();
        let label = match h.id {
            kfs::ID_MATERIAL_LIST => {
                f.material_lists.push(kfs::parse_material_list(&mut r, &h)?);
                "material list"
            }
            kfs::ID_MESH => {
                f.meshes.push(kfs::parse_mesh(&mut r, &h, &mut notes)?);
                "mesh"
            }
            skd::ID_SKIN => {
                f.skins.push(skd::parse_skin(&mut r, &h, &mut notes)?);
                "skin"
            }
            ID_KEYFRAME_ANIMATION => {
                f.tracks.push(parse_track(&mut r, &h, &mut notes)?);
                "keyframe animation"
            }
            ID_CAMERA => {
                f.cameras.push(parse_camera(&mut r, &mut notes)?);
                "camera"
            }
            ID_ENVIRONMENT => {
                f.environments.push(parse_environment(&mut r)?);
                "environment"
            }
            id => {
                let name = match id {
                    ID_POINT_LIGHT => "point light",
                    ID_DIRECTIONAL_LIGHT => "directional light",
                    ID_SPOT_LIGHT => "spot light",
                    ID_HELPER => "helper",
                    ID_POINT_LIGHT_ANIMATION => "point light animation",
                    ID_DIRECTIONAL_LIGHT_ANIMATION => "directional light animation",
                    ID_SPOT_LIGHT_ANIMATION => "spot light animation",
                    _ => "unknown",
                };
                let end = h.data_end();
                if end > data.len() {
                    return Err(ParseError {
                        offset: start,
                        msg: format!("{name} chunk 0x{id:08x} claims {} bytes but the file ends first", h.size),
                    });
                }
                r.seek(end)?;
                f.notes.push(format!("skipped {name} chunk 0x{id:08x} v{} ({} bytes) at 0x{start:x}", h.version, h.size));
                continue;
            }
        };
        kfs::check_size(&r, &h, label, &mut notes);
        f.notes.extend(notes);
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kfs::{ID_GEOMETRY, ID_MESH, ID_NODE, ID_POLYGONS};
    use crate::skd::ID_SKIN;
    use crate::testutil::*;

    fn track_chunk(version: u32, bone: &str, fps: i32, keys: &[(u32, [[f32; 3]; 4])], total: u32) -> Vec<u8> {
        let mut anim = string(bone);
        anim.extend(int(fps));
        anim.extend(boolean(true));
        let mut p = chunk(ID_ANIMATION, 0, &anim);
        p.extend(string("Bip01"));
        p.extend(boolean(false));
        p.extend(uint(total));
        p.extend(uint(keys.len() as u32));
        for (f, m) in keys {
            p.extend(uint(*f));
            p.extend(matrix(*m));
        }
        if version >= 1 {
            p.extend(uint(1));
            p.extend(uint(0));
            p.extend(float(1.0));
        }
        if version >= 2 {
            p.extend(uint(3));
        }
        if version >= 3 {
            p.extend(int(1));
        }
        if version >= 4 {
            p.extend(boolean(true));
        }
        chunk(ID_KEYFRAME_ANIMATION, version, &p)
    }

    fn node(name: &str, parent: &str) -> Vec<u8> {
        let mut p = string(name);
        p.extend(string(parent));
        p.extend(matrix(IDENT));
        p.extend(boolean(!parent.is_empty()));
        chunk(ID_NODE, 0, &p)
    }

    #[test]
    fn clip_from_tracks() {
        let mut moved = IDENT;
        moved[3] = [1.0, 2.0, 3.0];
        let mut file = track_chunk(5, "Bip01 Spine", 30, &[(0, IDENT), (29, moved)], 30);
        file.extend(track_chunk(1, "Bip01 Head", 30, &[(0, IDENT)], 9));
        let f = parse(&file).unwrap();
        assert_eq!(f.tracks.len(), 2);
        let t = &f.tracks[0];
        assert_eq!((t.object_name.as_str(), t.fps, t.total_frames, t.keys.len()), ("Bip01 Spine", 30, 30, 2));
        assert_eq!(t.keys[1].transform[3], [1.0, 2.0, 3.0]);
        assert_eq!(t.loop_to_frame, 3);
        assert!(t.maintain_matrix_scaling);
        // Version 1 stores one frame fewer than the real count.
        assert_eq!(f.tracks[1].total_frames, 10);
        assert_eq!(f.tracks[1].visibility, vec![(0, 1.0)]);
        let c = f.clip("run").unwrap();
        assert_eq!(c.frame_count, 30);
        assert!((c.duration_secs - 29.0 / 30.0).abs() < 1e-6);
        assert!(c.looping);
    }

    #[test]
    fn mixed_file_with_skipped_chunk() {
        let mut geo = uint(1);
        geo.extend(raw_vec3([1., 2., 3.]));
        geo.extend(raw_vec3([0., 0., 1.]));
        geo.extend(uint(1));
        geo.extend(uint(1));
        let mut polys = uint(0);
        polys.extend(uint(0));
        let mut mesh = node("Bip01", "");
        mesh.extend(chunk(ID_GEOMETRY, 1, &geo));
        mesh.extend(chunk(ID_POLYGONS, 1, &polys));
        let mut mesh2 = node("Bip01 Spine", "Bip01");
        mesh2.extend(chunk(ID_GEOMETRY, 1, &geo));

        let mut skin = int(0);
        skin.extend(int(0));
        skin.extend(int(0));
        skin.extend(int(1));
        skin.extend(string("body"));
        skin.extend(int(2));
        skin.extend(string("Bip01"));
        skin.extend(string("Bip01 Spine"));
        skin.extend(int(0));

        let mut file = chunk(ID_MESH, 2, &mesh);
        file.extend(chunk(0x0001_0002, 0, &[1, 2, 3, 4])); // point light, skipped
        file.extend(chunk(ID_MESH, 2, &mesh2));
        file.extend(chunk(ID_SKIN, 0, &skin));
        let mut env = Vec::new();
        for v in [10u8, 20, 30, 255, 1, 2, 3, 4] {
            env.extend([0x08, v]);
        }
        file.extend(chunk(ID_ENVIRONMENT, 0, &env));

        let f = parse(&file).unwrap();
        assert_eq!(f.meshes.len(), 2);
        assert_eq!(f.meshes[0].positions, vec![[1., 2., 3.]]);
        assert_eq!(f.skins[0].skeleton_objects.len(), 2);
        assert_eq!(f.environments[0].ambient, [10, 20, 30, 255]);
        assert!(f.notes.iter().any(|n| n.contains("point light")), "{:?}", f.notes);
        let sk = f.skeleton();
        assert_eq!(sk.bones.len(), 2);
        assert_eq!(sk.bones[1].parent, Some(0));
        assert!(f.clip("x").is_none());
    }

    #[test]
    fn truncated_file_is_error_not_panic() {
        let file = track_chunk(5, "Bip01", 30, &[(0, IDENT), (1, IDENT)], 2);
        for cut in 0..file.len() {
            // Every strict prefix must fail cleanly (or, for cut == 0, be an empty file).
            let r = parse(&file[..cut]);
            if cut == 0 {
                assert!(r.is_ok());
            } else {
                assert!(r.is_err(), "prefix of {cut} bytes parsed");
            }
        }
        assert!(parse(&file).is_ok());
    }

    #[test]
    fn garbage_is_error() {
        assert!(parse(&[0xFF; 40]).is_err());
        assert!(parse(&[0x0C, 1, 0]).is_err());
        // A chunk that claims more data than exists.
        let mut c = chunk(0x0001_0016, 0, &[]);
        c[9..13].copy_from_slice(&1000u32.to_le_bytes());
        assert!(parse(&c).is_err());
        // Unsupported version.
        assert!(parse(&chunk(ID_KEYFRAME_ANIMATION, 9, &[])).is_err());
    }
}
