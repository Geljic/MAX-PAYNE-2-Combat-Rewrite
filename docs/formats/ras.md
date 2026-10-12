# `.ras` resource archives

Max Payne 2 keeps its data in eight `MP2_*.ras` archives. Read by Remedy's `rlmfc.dll`
(`R_File`); worked out 2026-10-11 in Ghidra and checked by extracting all eight archives, with
every header, table and size check passing. Reader: `tools/extract/src/ras.rs`.

## Layout

| Part | Size | Scrambled |
|---|---|---|
| Header | 44 bytes | bytes 8–43 |
| File table | header `file_table_len` | yes |
| Directory table | header `dir_table_len` | yes |
| File bodies | sum of stored sizes | no |

All integers are little-endian `u32`.

### Header

| Offset | Field |
|---|---|
| 0x00 | magic `RAS\0` |
| 0x04 | seed for the scrambling (stored in the clear) |
| 0x08 | file count |
| 0x0C | directory count |
| 0x10 | file table length in bytes |
| 0x14 | directory table length in bytes |
| 0x18 | version, `f32`, must be 1.2 |
| 0x1C | CRC32 of the unscrambled 44-byte header with this field set to zero |
| 0x20 | CRC32 of the unscrambled file table |
| 0x24 | CRC32 of the unscrambled directory table |
| 0x28 | unknown (4 in every retail archive) |

The CRC is the standard zlib/IEEE CRC32.

### Scrambling

Each table is unscrambled on its own, with the byte index `i` starting at 0 for each one. For
each byte: rotate left by `i mod 5`, XOR with the low byte of `(i + 3) * 6`, then add the low byte
of the next value of a generator. The generator starts at the seed (1 if the seed is 0) and steps
as `s = 171·s − 30269·⌊s / 177⌋` in wrapping 32-bit signed arithmetic, with division truncating
toward zero.

### File table

One record per file: a NUL-terminated name, then 40 bytes:

| Offset after the name | Field |
|---|---|
| +0 | size in bytes once decompressed |
| +4 | stored size in the archive |
| +8 | flags (0 in retail) |
| +12 | directory index |
| +16, +20 | unknown (0 and 3 in retail) |
| +24 | 16 bytes, unknown (likely timestamps) |

File bodies follow the directory table directly, in file-table order. Each body starts where
the one before it ended, so offsets come from summing the stored sizes.

### Directory table

One record per directory: a NUL-terminated path such as `\data\database\weapons\`, then 16
unknown bytes. Directory 0 is the root `\`.

## Compression

When a file's size and stored size differ, the body is a block:

| Offset | Field |
|---|---|
| 0x00 | magic `RA->` |
| 0x04 | decompressed size |
| 0x08 | compressed payload length |
| 0x0C | payload |

The payload is classic Okumura LZSS:
- a 4096-byte ring buffer pre-filled with spaces, with writing starting at 0xFEE;
- one flag byte for every 8 items, read low bit first: a set bit means a literal byte, a clear
  bit means a 2-byte back-reference;
- a back-reference holds a 12-bit position (low byte plus the high nibble of the second byte)
  and a length of the second byte's low nibble plus 3.

Only the level files (`.ldb` in `MP2_Levels_*.ras`) are compressed in the retail game.

## What is where

| Archive | Contents |
|---|---|
| MP2_Init | the database's shared headers and tables (`shooting.h`, `materials.txt`, `defaultvalues.txt`, …) |
| MP2_Data | the database: skins, skeletons and 1296 male animations, weapons, ragdolls, projectiles, HUD |
| MP2_Levels_A/B/C | levels (`.ldb`, compressed) |
| MP2_Sounds, MP2_Music, MP2_English | audio, localisation, subtitles |
