//! Reader for Max Payne 2's `.ras` resource archives. Format notes: `docs/formats/ras.md`.
//!
//! Layout: a 44-byte header (magic, seed, then 36 scrambled bytes), a scrambled file table,
//! a scrambled directory table, then the file bodies back to back in table order. Each table
//! carries a CRC32. Bodies are stored as-is, except some (the levels) are LZSS blocks.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"RAS\0";
const HEADER_LEN: usize = 0x2c;
const VERSION: f32 = 1.2;
const LZSS_MAGIC: &[u8; 4] = b"RA->";

pub struct Entry {
    /// Path inside the archive with forward slashes, e.g. `data/database/weapons.txt`.
    pub path: String,
    pub size: u32,
    pub stored: u32,
    pub offset: u64,
}

pub struct Archive {
    file: File,
    pub entries: Vec<Entry>,
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Undoes the archive's byte scrambling: each byte is rotated left by `i % 5`, then XORed with
/// a position pattern and offset by a Park-Miller style generator (Schrage's method, a = 171,
/// m = 30269) seeded from the header.
pub fn unscramble(buf: &mut [u8], seed: u32) {
    let mut s = if seed == 0 { 1i32 } else { seed as i32 };
    for (i, b) in buf.iter_mut().enumerate() {
        let x = b.rotate_left((i % 5) as u32);
        s = s.wrapping_mul(171).wrapping_sub((s / 177).wrapping_mul(30269));
        *b = ((((i as u32 + 3) * 6) as u8) ^ x).wrapping_add(s as u8);
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| bad(format!("table truncated at {at:#x}")))
}

fn cstr_at(b: &[u8], at: usize) -> io::Result<(String, usize)> {
    let end = b[at.min(b.len())..]
        .iter()
        .position(|&c| c == 0)
        .map(|n| at + n)
        .ok_or_else(|| bad(format!("unterminated name at {at:#x}")))?;
    Ok((b[at..end].iter().map(|&c| c as char).collect(), end + 1))
}

impl Archive {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut h = [0u8; HEADER_LEN];
        file.read_exact(&mut h)?;
        if &h[..4] != MAGIC {
            return Err(bad("not a RAS archive"));
        }
        let seed = u32::from_le_bytes(h[4..8].try_into().unwrap());
        unscramble(&mut h[8..], seed);
        let f = |at| u32::from_le_bytes(h[at..at + 4].try_into().unwrap());
        let (n_files, n_dirs, ft_len, dt_len) = (f(0x08), f(0x0c), f(0x10) as usize, f(0x14) as usize);
        let version = f32::from_bits(f(0x18));
        let (hdr_crc, ft_crc, dt_crc) = (f(0x1c), f(0x20), f(0x24));
        let mut check = h;
        check[0x1c..0x20].fill(0);
        if crc32(&check) != hdr_crc {
            return Err(bad("header CRC mismatch"));
        }
        if version != VERSION {
            return Err(bad(format!("archive version {version}, expected {VERSION}")));
        }

        let mut table = |len: usize, crc: u32, what: &str| -> io::Result<Vec<u8>> {
            let mut t = vec![0u8; len];
            file.read_exact(&mut t)?;
            unscramble(&mut t, seed);
            if crc32(&t) != crc {
                return Err(bad(format!("{what} table CRC mismatch")));
            }
            Ok(t)
        };
        let ft = table(ft_len, ft_crc, "file")?;
        let dt = table(dt_len, dt_crc, "directory")?;

        // Directory entry: name, then 16 bytes we don't need.
        let mut dirs = Vec::with_capacity(n_dirs as usize);
        let mut p = 0;
        for _ in 0..n_dirs {
            let (name, next) = cstr_at(&dt, p)?;
            dirs.push(name.trim_matches('\\').replace('\\', "/"));
            p = next + 16;
        }

        // File entry: name, then size, stored size, flags, directory index, two words and 16
        // bytes we don't need. Bodies follow the tables in entry order.
        let mut offset = file.stream_position()?;
        let mut entries = Vec::with_capacity(n_files as usize);
        let mut p = 0;
        for _ in 0..n_files {
            let (name, next) = cstr_at(&ft, p)?;
            let (size, stored, dir) = (u32_at(&ft, next)?, u32_at(&ft, next + 4)?, u32_at(&ft, next + 12)?);
            let dir = dirs.get(dir as usize).ok_or_else(|| bad(format!("{name}: bad directory index {dir}")))?;
            let path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
            entries.push(Entry { path, size, stored, offset });
            offset += stored as u64;
            p = next + 40;
        }
        Ok(Self { file, entries })
    }

    /// The file's contents, decompressed if needed.
    pub fn read(&mut self, i: usize) -> io::Result<Vec<u8>> {
        let e = &self.entries[i];
        let mut raw = vec![0u8; e.stored as usize];
        self.file.seek(SeekFrom::Start(e.offset))?;
        self.file.read_exact(&mut raw)?;
        if e.size == e.stored {
            return Ok(raw);
        }
        if raw.len() < 12 || &raw[..4] != LZSS_MAGIC {
            return Err(bad(format!("{}: sizes differ but no LZSS block", e.path)));
        }
        let out_len = u32_at(&raw, 4)? as usize;
        let in_len = u32_at(&raw, 8)? as usize;
        let body = raw.get(12..12 + in_len).ok_or_else(|| bad(format!("{}: LZSS block truncated", e.path)))?;
        let out = lzss(body, out_len);
        if out.len() != e.size as usize {
            return Err(bad(format!("{}: decompressed {} bytes, expected {}", e.path, out.len(), e.size)));
        }
        Ok(out)
    }
}

/// Okumura-style LZSS: 4 KB ring buffer pre-filled with spaces, writing from 0xFEE; a flag byte
/// per 8 items, bit set = literal, clear = 2-byte back-reference (12-bit position, 4-bit length + 3).
pub fn lzss(src: &[u8], out_len: usize) -> Vec<u8> {
    let mut ring = [b' '; 4096];
    let mut r = 0xFEE;
    let mut out = Vec::with_capacity(out_len);
    let mut i = 0;
    let mut flags = 0u32;
    loop {
        flags >>= 1;
        if flags & 0x100 == 0 {
            let Some(&b) = src.get(i) else { break };
            i += 1;
            flags = b as u32 | 0xFF00;
        }
        if flags & 1 != 0 {
            let Some(&b) = src.get(i) else { break };
            i += 1;
            out.push(b);
            ring[r] = b;
            r = (r + 1) & 0xFFF;
        } else {
            let (Some(&lo), Some(&hi)) = (src.get(i), src.get(i + 1)) else { break };
            i += 2;
            let pos = lo as usize | ((hi as usize & 0xF0) << 4);
            for k in 0..(hi as usize & 0xF) + 3 {
                let b = ring[(pos + k) & 0xFFF];
                out.push(b);
                ring[r] = b;
                r = (r + 1) & 0xFFF;
            }
        }
    }
    out
}

/// Extracts every file of `archive` under `out`, returning the number of files written.
pub fn extract(archive: &Path, out: &Path) -> io::Result<usize> {
    let mut a = Archive::open(archive)?;
    for i in 0..a.entries.len() {
        let path: PathBuf = out.join(&a.entries[i].path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, a.read(i)?)?;
    }
    Ok(a.entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn lzss_literals_and_backref() {
        // flags 0b0000_0111: three literals "abc", then a back-reference to ring 0xFEE, length 6.
        let src = [0x07, b'a', b'b', b'c', 0xEE, 0xF3];
        assert_eq!(lzss(&src, 9), b"abcabcabc");
    }

    #[test]
    fn lzss_reads_initial_spaces() {
        // A back-reference into the untouched ring yields spaces.
        assert_eq!(lzss(&[0x00, 0x00, 0x00], 3), b"   ");
    }
}
