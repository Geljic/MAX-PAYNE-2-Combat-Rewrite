//! Builders for synthetic Max Payne data, used only by tests.

pub const IDENT: [[f32; 3]; 4] = crate::xform::IDENTITY;

pub fn int(v: i32) -> Vec<u8> {
    let mut b = vec![0x02];
    b.extend(v.to_le_bytes());
    b
}

pub fn uint(v: u32) -> Vec<u8> {
    let mut b = vec![0x03];
    b.extend(v.to_le_bytes());
    b
}

pub fn float(v: f32) -> Vec<u8> {
    let mut b = vec![0x09];
    b.extend(v.to_le_bytes());
    b
}

pub fn boolean(v: bool) -> Vec<u8> {
    vec![0x0E, v as u8]
}

pub fn string(s: &str) -> Vec<u8> {
    let mut b = vec![0x0D];
    b.extend(int(s.len() as i32));
    b.extend(s.as_bytes());
    b
}

pub fn vec3(v: [f32; 3]) -> Vec<u8> {
    let mut b = vec![0x16];
    b.extend(raw_vec3(v));
    b
}

pub fn raw_vec3(v: [f32; 3]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn matrix(m: [[f32; 3]; 4]) -> Vec<u8> {
    let mut b = vec![0x1A];
    for row in m {
        b.extend(raw_vec3(row));
    }
    b
}

pub fn chunk(id: u32, version: u32, payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0x0C];
    b.extend(id.to_le_bytes());
    b.extend(version.to_le_bytes());
    b.extend(((payload.len() + 13) as u32).to_le_bytes());
    b.extend(payload);
    b
}
