//! A small little-endian byte reader for Max Payne's tagged data format.
//!
//! Every read either returns a value or a [`ParseError`] carrying the byte
//! offset and what was expected, so a bad file never panics and the message
//! says where to look in a hex editor.
//!
//! Max Payne stores most values as `{tag}{data}` (see `docs/formats/kf2.md`).
//! The typed readers here (`int`, `uint`, `float`, `boolean`, `string`, `vec3`,
//! `matrix4x3`) check the tag and accept every width the format defines.

use std::fmt;

/// A parse failure: where it happened and what was wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    /// Byte offset into the input where the problem was detected.
    pub offset: usize,
    /// Human-readable description, e.g. "expected M_Int for `vertex count`".
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at offset 0x{:x}: {}", self.offset, self.msg)
    }
}

impl std::error::Error for ParseError {}

pub type Result<T> = std::result::Result<T, ParseError>;

/// First byte of every chunk header.
pub const CHUNK_TAG: u8 = 0x0C;
/// Size of a chunk header: tag + id + version + size.
pub const CHUNK_HEADER_LEN: usize = 13;

/// A chunk header (`0x0C`, id, version, data size).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHeader {
    pub id: u32,
    pub version: u32,
    /// Declared size of the whole chunk in bytes, header included (checked
    /// against real files; the spec calls it the data size).
    pub size: u32,
    /// Offset of the first data byte, right after the header.
    pub data_start: usize,
}

impl ChunkHeader {
    /// Offset one past the end of the chunk, saturating on overflow.
    pub fn data_end(&self) -> usize {
        self.data_start.saturating_sub(CHUNK_HEADER_LEN).saturating_add(self.size as usize)
    }
}

/// Cursor over a byte slice.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Build an error at the current offset.
    pub fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError { offset: self.pos, msg: msg.into() }
    }

    /// Move the cursor, which may point at most one past the end.
    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(self.err(format!("cannot seek to 0x{pos:x}, file is 0x{:x} bytes", self.data.len())));
        }
        self.pos = pos;
        Ok(())
    }

    /// Read `n` raw bytes.
    pub fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(self.err(format!(
                "unexpected end of data: expected {n} bytes for {what}, only {} left",
                self.remaining()
            )));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize, what: &str) -> Result<()> {
        self.take(n, what).map(|_| ())
    }

    pub fn peek_u8(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    pub fn u8_raw(&mut self, what: &str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    pub fn u16_raw(&mut self, what: &str) -> Result<u16> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32_raw(&mut self, what: &str) -> Result<u32> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn f32_raw(&mut self, what: &str) -> Result<f32> {
        Ok(f32::from_bits(self.u32_raw(what)?))
    }

    /// Three untagged floats (used by the version 1 geometry/UV arrays).
    pub fn vec3_raw(&mut self, what: &str) -> Result<[f32; 3]> {
        Ok([self.f32_raw(what)?, self.f32_raw(what)?, self.f32_raw(what)?])
    }

    // ----- chunk headers -------------------------------------------------

    /// Read a 13-byte chunk header.
    pub fn chunk_header(&mut self, what: &str) -> Result<ChunkHeader> {
        let at = self.pos;
        let tag = self.u8_raw(what)?;
        if tag != CHUNK_TAG {
            self.pos = at;
            return Err(self.err(format!("expected chunk tag 0x0C for {what}, found 0x{tag:02x}")));
        }
        let id = self.u32_raw(what)?;
        let version = self.u32_raw(what)?;
        let size = self.u32_raw(what)?;
        Ok(ChunkHeader { id, version, size, data_start: self.pos })
    }

    /// The id of the chunk starting at the cursor, without consuming it.
    pub fn peek_chunk_id(&self) -> Option<u32> {
        let rest = self.data.get(self.pos..)?;
        if rest.len() < CHUNK_HEADER_LEN || rest[0] != CHUNK_TAG {
            return None;
        }
        Some(u32::from_le_bytes([rest[1], rest[2], rest[3], rest[4]]))
    }

    // ----- tagged values -------------------------------------------------

    fn tag(&mut self, what: &str, expected: &str) -> Result<u8> {
        let at = self.pos;
        match self.u8_raw(what) {
            Ok(t) => Ok(t),
            Err(_) => {
                self.pos = at;
                Err(self.err(format!("unexpected end of data: expected {expected} for {what}")))
            }
        }
    }

    /// Any of the integer tags (`M_Int`, `M_Unsigned_Int`, bool, char, short,
    /// 3/2/1-byte forms), widened to `i64`.
    pub fn int(&mut self, what: &str) -> Result<i64> {
        let at = self.pos;
        let tag = self.tag(what, "M_Int")?;
        let v = match tag {
            0x00 | 0x02 => i32::from_le_bytes(self.take(4, what)?.try_into().unwrap()) as i64,
            0x01 | 0x03 => u32::from_le_bytes(self.take(4, what)?.try_into().unwrap()) as i64,
            0x04 | 0x13 => i16::from_le_bytes(self.take(2, what)?.try_into().unwrap()) as i64,
            0x05 | 0x10 => u16::from_le_bytes(self.take(2, what)?.try_into().unwrap()) as i64,
            0x06 | 0x07 | 0x14 => self.take(1, what)?[0] as i8 as i64,
            0x08 | 0x0E | 0x11 => self.take(1, what)?[0] as i64,
            0x0F => {
                let b = self.take(3, what)?;
                b[0] as i64 | (b[1] as i64) << 8 | (b[2] as i64) << 16
            }
            0x12 => {
                let b = self.take(3, what)?;
                let v = b[0] as i64 | (b[1] as i64) << 8 | (b[2] as i64) << 16;
                if v & 0x80_0000 != 0 { v - 0x100_0000 } else { v }
            }
            t => {
                self.pos = at;
                return Err(self.err(format!("expected M_Int for {what}, found tag 0x{t:02x}")));
            }
        };
        Ok(v)
    }

    /// An integer that must fit in `u32`.
    pub fn uint(&mut self, what: &str) -> Result<u32> {
        let at = self.pos;
        let v = self.int(what)?;
        u32::try_from(v).map_err(|_| ParseError {
            offset: at,
            msg: format!("expected non-negative 32-bit integer for {what}, got {v}"),
        })
    }

    /// An element count, checked so that `count * min_elem_bytes` cannot
    /// exceed what is left in the file. This stops a corrupt count from
    /// causing a huge allocation.
    pub fn count(&mut self, what: &str, min_elem_bytes: usize) -> Result<usize> {
        let at = self.pos;
        let n = self.uint(what)? as usize;
        if n.saturating_mul(min_elem_bytes.max(1)) > self.remaining() {
            return Err(ParseError {
                offset: at,
                msg: format!(
                    "{what} is {n}, which cannot fit in the {} bytes left",
                    self.remaining()
                ),
            });
        }
        Ok(n)
    }

    /// `M_Float` (tag 0x09), `M_Double` (0x0A) or the 16-bit float (0x26).
    pub fn float(&mut self, what: &str) -> Result<f32> {
        let at = self.pos;
        let tag = self.tag(what, "M_Float")?;
        match tag {
            0x09 => self.f32_raw(what),
            0x0A => Ok(f64::from_le_bytes(self.take(8, what)?.try_into().unwrap()) as f32),
            0x26 => Ok(half_to_f32(self.u16_raw(what)?)),
            t => {
                self.pos = at;
                Err(self.err(format!("expected M_Float for {what}, found tag 0x{t:02x}")))
            }
        }
    }

    /// `M_Bool` (any integer tag is accepted, non-zero is true).
    pub fn boolean(&mut self, what: &str) -> Result<bool> {
        Ok(self.int(what)? != 0)
    }

    /// `M_String`: tag 0x0D, an `M_Int` length, then that many bytes. The
    /// bytes are read as Latin-1 so odd names never fail; trailing NULs are
    /// trimmed.
    pub fn string(&mut self, what: &str) -> Result<String> {
        let at = self.pos;
        let tag = self.tag(what, "M_String")?;
        if tag != 0x0D {
            self.pos = at;
            return Err(self.err(format!("expected M_String (tag 0x0D) for {what}, found tag 0x{tag:02x}")));
        }
        let n = self.count(what, 1)?;
        let bytes = self.take(n, what)?;
        let s: String = bytes.iter().map(|&b| b as char).collect();
        Ok(s.trim_end_matches('\0').to_string())
    }

    fn expect_tag(&mut self, want: u8, name: &str, what: &str) -> Result<()> {
        let at = self.pos;
        let tag = self.tag(what, name)?;
        if tag != want {
            self.pos = at;
            return Err(self.err(format!("expected {name} (tag 0x{want:02x}) for {what}, found tag 0x{tag:02x}")));
        }
        Ok(())
    }

    /// `M_Vector3` (tag 0x16).
    pub fn vec3(&mut self, what: &str) -> Result<[f32; 3]> {
        self.expect_tag(0x16, "M_Vector3", what)?;
        self.vec3_raw(what)
    }

    /// `M_Matrix4x3` (tag 0x1A): four rows of three floats.
    pub fn matrix4x3(&mut self, what: &str) -> Result<[[f32; 3]; 4]> {
        self.expect_tag(0x1A, "M_Matrix4x3", what)?;
        let mut m = [[0.0; 3]; 4];
        for row in &mut m {
            *row = self.vec3_raw(what)?;
        }
        Ok(m)
    }
}

/// IEEE 754 half precision to single.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let exp = ((h >> 10) & 0x1F) as i32;
    let frac = (h & 0x03FF) as f32;
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        0x1F => {
            if frac == 0.0 { sign * f32::INFINITY } else { f32::NAN }
        }
        e => sign * (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_widths() {
        let data = [0x02, 0xFE, 0xFF, 0xFF, 0xFF, 0x12, 0xFF, 0xFF, 0xFF, 0x14, 0xFF, 0x0F, 1, 2, 3];
        let mut r = Reader::new(&data);
        assert_eq!(r.int("a").unwrap(), -2);
        assert_eq!(r.int("b").unwrap(), -1);
        assert_eq!(r.int("c").unwrap(), -1);
        assert_eq!(r.int("d").unwrap(), 0x030201);
        assert!(r.is_empty());
    }

    #[test]
    fn half_floats() {
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0xC000), -2.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        let data = [0x26, 0x00, 0x3C];
        assert_eq!(Reader::new(&data).float("x").unwrap(), 1.0);
    }

    #[test]
    fn errors_carry_offset() {
        let data = [0x02, 0x01];
        let e = Reader::new(&data).int("vertex count").unwrap_err();
        assert_eq!(e.offset, 1);
        assert!(e.msg.contains("vertex count"), "{e}");

        let data = [0x09, 0, 0, 0, 0];
        let e = Reader::new(&data).int("n").unwrap_err();
        assert_eq!(e.offset, 0);
        assert!(e.msg.contains("0x09"));
    }

    #[test]
    fn huge_count_rejected() {
        let data = [0x03, 0xFF, 0xFF, 0xFF, 0x7F, 0, 0];
        assert!(Reader::new(&data).count("items", 1).is_err());
    }

    #[test]
    fn string_roundtrip() {
        let data = [0x0D, 0x11, 3, b'a', b'b', 0];
        assert_eq!(Reader::new(&data).string("s").unwrap(), "ab");
        let short = [0x0D, 0x11, 9, b'a'];
        assert!(Reader::new(&short).string("s").is_err());
    }
}
