//! Up to four fixed-size REU volumes with a replicated, little-endian directory.
//!
//! Header: magic[8], version:u16, header_bytes:u16, volume:u8, count:u8,
//! entries:u16, volume_bytes:u32, payload_end:u32, directory_crc32:u32,
//! payload_crc32:u32, header_crc32:u32, reserved[28]. The header CRC covers
//! all 4096 bytes with its own four bytes zeroed. Directory entries start at 64.
//! Each entry: id:u16, kind:u8, volume:u8, offset:u32, length:u32, crc32:u32,
//! metadata:[u32;4], UTF-8 name[32] (zero padded). Offsets address the file.
//! CRCs use CRC-32/ISO-HDLC; they detect corruption, not malicious modification.
//! Version 2 retains this directory layout but changes panorama payloads to a
//! 32 KiB center cache followed by 16x8 native raster tiles. Track metadata is
//! [sample_rate, stereo_frames, channels, code_bits]; panorama metadata is
//! [logical_width, logical_height, padded_width, padded_height].

use std::collections::BTreeSet;
use std::fmt;

/// The packer writes as many 16 MiB volumes as the content needs, up to this many.
pub const MAX_VOLUME_COUNT: usize = 4;
pub const VOLUME_BYTES: usize = 16 * 1024 * 1024;
pub const HEADER_BYTES: usize = 4096;
pub const PAYLOAD_BYTES: usize = VOLUME_BYTES - HEADER_BYTES;
pub const TOTAL_PAYLOAD_BYTES: usize = PAYLOAD_BYTES * MAX_VOLUME_COUNT;
pub const DIRECTORY_OFFSET: usize = 64;
pub const ENTRY_BYTES: usize = 64;
pub const MAX_ASSETS: usize = 42;
pub const ALIGNMENT: usize = 256;
pub const FORMAT_VERSION: u16 = 2;
pub const MAGIC: &[u8; 8] = b"HVMDVOL\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum AssetKind {
    Track = 1,
    Panorama = 2,
}

#[derive(Clone, Debug)]
pub struct AssetInput {
    pub id: u16,
    pub kind: AssetKind,
    pub name: String,
    pub data: Vec<u8>,
    /// Kind-specific information; packaging preserves all four words verbatim.
    pub metadata: [u32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetEntry {
    pub id: u16,
    pub kind: AssetKind,
    pub volume: u8,
    pub offset: u32,
    pub length: u32,
    pub crc32: u32,
    pub metadata: [u32; 4],
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeHeader {
    pub volume_index: u8,
    pub volume_count: u8,
    pub payload_end: u32,
    pub payload_crc32: u32,
    pub entries: Vec<AssetEntry>,
}

pub struct VolumeSet {
    pub volumes: Vec<Vec<u8>>,
    pub entries: Vec<AssetEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError(pub String);
impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for FormatError {}
type Result<T> = std::result::Result<T, FormatError>;
fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(FormatError(message.into()))
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}
const CRC_TABLE: [u32; 256] = crc_table();

/// Incremental CRC state; use `finish` only after the complete asset arrives.
#[derive(Clone, Debug)]
pub struct Crc32(u32);
impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}
impl Crc32 {
    pub fn new() -> Self {
        Self(u32::MAX)
    }
    pub fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 >> 8) ^ CRC_TABLE[((self.0 as u8) ^ byte) as usize];
        }
    }
    pub fn finish(&self) -> u32 {
        !self.0
    }
}
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(bytes);
    crc.finish()
}
fn header_crc(bytes: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(&bytes[..32]);
    crc.update(&[0; 4]);
    crc.update(&bytes[36..HEADER_BYTES]);
    crc.finish()
}
fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn get16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn get32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn align(value: usize) -> usize {
    (value + ALIGNMENT - 1) & !(ALIGNMENT - 1)
}

/// Deterministic kind/id packing in a logical stream spanning up to four payloads;
/// only the volumes the content reaches are written, and every header names the count.
/// Assets may cross volume boundaries; skip each next volume's 4 KiB header.
/// All files are exactly 16 MiB; every file contains the complete global directory.
pub fn pack(assets: &[AssetInput]) -> Result<VolumeSet> {
    if assets.is_empty() || assets.len() > MAX_ASSETS {
        return fail("expected 1..=42 assets");
    }
    let mut keys = BTreeSet::new();
    for asset in assets {
        if !keys.insert((asset.kind, asset.id)) {
            return fail("duplicate asset kind/id");
        }
        if asset.name.len() > 32 || asset.name.contains('\0') {
            return fail("asset name must be at most 32 UTF-8 bytes without NUL");
        }
        if asset.data.is_empty() || asset.data.len() > TOTAL_PAYLOAD_BYTES {
            return fail(format!(
                "asset {} exceeds total volume capacity",
                asset.name
            ));
        }
    }
    let mut order: Vec<_> = assets.iter().collect();
    order.sort_by_key(|a| (a.kind, a.id));
    let mut volumes: Vec<Vec<u8>> = Vec::new();
    let mut ends: Vec<usize> = Vec::new();
    let mut entries = Vec::with_capacity(assets.len());
    let mut cursor = 0usize;
    for asset in order {
        cursor = align(cursor);
        if cursor + asset.data.len() > TOTAL_PAYLOAD_BYTES {
            return fail("assets exceed four-volume capacity");
        }
        let volume = cursor / PAYLOAD_BYTES;
        let offset = HEADER_BYTES + cursor % PAYLOAD_BYTES;
        let mut consumed = 0;
        while consumed < asset.data.len() {
            let index = cursor / PAYLOAD_BYTES;
            let at = HEADER_BYTES + cursor % PAYLOAD_BYTES;
            let count = (asset.data.len() - consumed).min(VOLUME_BYTES - at);
            while volumes.len() <= index {
                volumes.push(vec![0u8; VOLUME_BYTES]);
                ends.push(HEADER_BYTES);
            }
            volumes[index][at..at + count].copy_from_slice(&asset.data[consumed..consumed + count]);
            ends[index] = at + count;
            cursor += count;
            consumed += count;
        }
        entries.push(AssetEntry {
            id: asset.id,
            kind: asset.kind,
            volume: volume as u8,
            offset: offset as u32,
            length: asset.data.len() as u32,
            crc32: crc32(&asset.data),
            metadata: asset.metadata,
            name: asset.name.clone(),
        });
    }
    entries.sort_by_key(|a| (a.kind, a.id));
    let mut directory = [0u8; HEADER_BYTES - DIRECTORY_OFFSET];
    for (i, entry) in entries.iter().enumerate() {
        let record = &mut directory[i * ENTRY_BYTES..(i + 1) * ENTRY_BYTES];
        put16(record, 0, entry.id);
        record[2] = entry.kind as u8;
        record[3] = entry.volume;
        put32(record, 4, entry.offset);
        put32(record, 8, entry.length);
        put32(record, 12, entry.crc32);
        for (j, value) in entry.metadata.iter().enumerate() {
            put32(record, 16 + j * 4, *value);
        }
        record[32..32 + entry.name.len()].copy_from_slice(entry.name.as_bytes());
    }
    let volume_count = volumes.len() as u8;
    for (i, volume) in volumes.iter_mut().enumerate() {
        let payload_crc = crc32(&volume[HEADER_BYTES..ends[i]]);
        volume[..8].copy_from_slice(MAGIC);
        put16(volume, 8, FORMAT_VERSION);
        put16(volume, 10, HEADER_BYTES as u16);
        volume[12] = i as u8;
        volume[13] = volume_count;
        put16(volume, 14, entries.len() as u16);
        put32(volume, 16, VOLUME_BYTES as u32);
        put32(volume, 20, ends[i] as u32);
        put32(volume, 24, crc32(&directory));
        put32(volume, 28, payload_crc);
        volume[DIRECTORY_OFFSET..HEADER_BYTES].copy_from_slice(&directory);
        let checksum = header_crc(volume);
        put32(volume, 32, checksum);
    }
    Ok(VolumeSet { volumes, entries })
}

/// Validate a 4 KiB directory before issuing any file or REU reads.
pub fn parse_header(bytes: &[u8]) -> Result<VolumeHeader> {
    if bytes.len() < HEADER_BYTES {
        return fail("short volume header");
    }
    if &bytes[..8] != MAGIC {
        return fail("invalid volume magic");
    }
    if get16(bytes, 8) != FORMAT_VERSION {
        return fail("unsupported volume version");
    }
    if get16(bytes, 10) as usize != HEADER_BYTES || get32(bytes, 16) as usize != VOLUME_BYTES {
        return fail("invalid volume dimensions");
    }
    let count = get16(bytes, 14) as usize;
    if count == 0 || count > MAX_ASSETS {
        return fail("invalid directory entry count");
    }
    if header_crc(bytes) != get32(bytes, 32) {
        return fail("header checksum mismatch");
    }
    let volume_count = bytes[13] as usize;
    if volume_count == 0 || volume_count > MAX_VOLUME_COUNT || bytes[12] as usize >= volume_count {
        return fail("invalid volume index/count");
    }
    if crc32(&bytes[DIRECTORY_OFFSET..HEADER_BYTES]) != get32(bytes, 24) {
        return fail("directory checksum mismatch");
    }
    let payload_end = get32(bytes, 20);
    if !(HEADER_BYTES as u32..=VOLUME_BYTES as u32).contains(&payload_end) {
        return fail("invalid payload extent");
    }
    let mut entries: Vec<AssetEntry> = Vec::with_capacity(count);
    let mut keys = BTreeSet::new();
    for i in 0..count {
        let record =
            &bytes[DIRECTORY_OFFSET + i * ENTRY_BYTES..DIRECTORY_OFFSET + (i + 1) * ENTRY_BYTES];
        let kind = match record[2] {
            1 => AssetKind::Track,
            2 => AssetKind::Panorama,
            _ => return fail("unknown asset kind"),
        };
        let id = get16(record, 0);
        if !keys.insert((kind, id)) {
            return fail("duplicate directory kind/id");
        }
        let volume = record[3];
        let offset = get32(record, 4);
        let length = get32(record, 8);
        let start = volume as u64 * PAYLOAD_BYTES as u64
            + offset.saturating_sub(HEADER_BYTES as u32) as u64;
        let end = start + length as u64;
        if volume as usize >= volume_count
            || length == 0
            || offset < HEADER_BYTES as u32
            || !(offset as usize).is_multiple_of(ALIGNMENT)
            || offset >= VOLUME_BYTES as u32
            || end > (volume_count * PAYLOAD_BYTES) as u64
        {
            return fail("invalid asset extent");
        }
        let local_start = bytes[12] as u64 * PAYLOAD_BYTES as u64;
        let local_end = local_start + PAYLOAD_BYTES as u64;
        if start < local_end
            && end > local_start
            && (end.min(local_end) - local_start + HEADER_BYTES as u64) > payload_end as u64
        {
            return fail("asset exceeds volume payload");
        }
        for previous in &entries {
            let previous_start = previous.volume as u64 * PAYLOAD_BYTES as u64
                + (previous.offset - HEADER_BYTES as u32) as u64;
            if start < previous_start + previous.length as u64 && previous_start < end {
                return fail("overlapping asset extents");
            }
        }
        let name_bytes = &record[32..64];
        let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
        if name_bytes[name_end..].iter().any(|&b| b != 0) {
            return fail("invalid name padding");
        }
        let name = std::str::from_utf8(&name_bytes[..name_end])
            .map_err(|_| FormatError("invalid UTF-8 asset name".into()))?
            .to_owned();
        entries.push(AssetEntry {
            id,
            kind,
            volume,
            offset,
            length,
            crc32: get32(record, 12),
            metadata: std::array::from_fn(|j| get32(record, 16 + j * 4)),
            name,
        });
    }
    Ok(VolumeHeader {
        volume_index: bytes[12],
        volume_count: bytes[13],
        payload_end,
        payload_crc32: get32(bytes, 28),
        entries,
    })
}

/// Validate an entire file, including each local asset and unused zero padding.
pub fn validate_volume(bytes: &[u8]) -> Result<VolumeHeader> {
    if bytes.len() != VOLUME_BYTES {
        return fail("volume file must be exactly 16 MiB");
    }
    let header = parse_header(bytes)?;
    if crc32(&bytes[HEADER_BYTES..header.payload_end as usize]) != header.payload_crc32 {
        return fail("volume payload checksum mismatch");
    }
    for entry in header.entries.iter().filter(|entry| {
        entry.volume == header.volume_index && entry.length <= VOLUME_BYTES as u32 - entry.offset
    }) {
        if crc32(&bytes[entry.offset as usize..(entry.offset + entry.length) as usize])
            != entry.crc32
        {
            return fail(format!("asset {} checksum mismatch", entry.id));
        }
    }
    if bytes[header.payload_end as usize..].iter().any(|&b| b != 0) {
        return fail("nonzero trailing volume padding");
    }
    Ok(header)
}

/// Refuse mixed releases even when each individual file has valid checksums.
pub fn validate_set(volumes: &[&[u8]]) -> Result<Vec<AssetEntry>> {
    if volumes.is_empty() || volumes.len() > MAX_VOLUME_COUNT {
        return fail("one to four volumes required");
    }
    let mut entries = None;
    let mut indexes = BTreeSet::new();
    let mut ordered = vec![None; volumes.len()];
    for volume in volumes {
        let header = validate_volume(volume)?;
        if header.volume_count as usize != volumes.len() {
            return fail("volume count does not match the set");
        }
        if !indexes.insert(header.volume_index) {
            return fail("duplicate volume index");
        }
        ordered[header.volume_index as usize] = Some(*volume);
        if let Some(expected) = &entries {
            if expected != &header.entries {
                return fail("volume directories do not match");
            }
        } else {
            entries = Some(header.entries);
        }
    }
    let entries = entries.unwrap();
    for entry in &entries {
        let mut crc = Crc32::new();
        let mut position =
            entry.volume as usize * PAYLOAD_BYTES + entry.offset as usize - HEADER_BYTES;
        let mut remaining = entry.length as usize;
        while remaining > 0 {
            let index = position / PAYLOAD_BYTES;
            let at = HEADER_BYTES + position % PAYLOAD_BYTES;
            let count = remaining.min(VOLUME_BYTES - at);
            crc.update(&ordered[index].unwrap()[at..at + count]);
            position += count;
            remaining -= count;
        }
        if crc.finish() != entry.crc32 {
            return fail(format!("asset {} checksum mismatch", entry.id));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn asset(id: u16, kind: AssetKind, bytes: Vec<u8>) -> AssetInput {
        AssetInput {
            id,
            kind,
            name: format!("asset-{id}"),
            data: bytes,
            metadata: [1, 2, 3, 4],
        }
    }
    fn resign(bytes: &mut [u8]) {
        let directory = crc32(&bytes[DIRECTORY_OFFSET..HEADER_BYTES]);
        put32(bytes, 24, directory);
        let value = header_crc(bytes);
        put32(bytes, 32, value);
    }
    #[test]
    fn standard_crc_and_incremental_agree() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let mut c = Crc32::new();
        c.update(b"1234");
        c.update(b"56789");
        assert_eq!(c.finish(), crc32(b"123456789"));
    }
    #[test]
    fn four_volumes_roundtrip_and_detect_corruption() {
        let source = [
            asset(0, AssetKind::Track, vec![12; 800]),
            asset(0, AssetKind::Panorama, vec![23; 503]),
            asset(1, AssetKind::Track, vec![41; 300]),
        ];
        let mut packed = pack(&source).unwrap();
        assert_eq!(packed.volumes.len(), 1);
        assert_eq!(
            validate_set(&packed.volumes.iter().map(Vec::as_slice).collect::<Vec<_>>()).unwrap(),
            packed.entries
        );
        for input in &source {
            let e = packed
                .entries
                .iter()
                .find(|e| e.id == input.id && e.kind == input.kind)
                .unwrap();
            assert_eq!(
                &packed.volumes[e.volume as usize]
                    [e.offset as usize..(e.offset + e.length) as usize],
                &input.data
            );
        }
        let e = &packed.entries[0];
        packed.volumes[e.volume as usize][e.offset as usize] ^= 1;
        assert!(
            validate_volume(&packed.volumes[e.volume as usize])
                .unwrap_err()
                .0
                .contains("checksum")
        );
    }
    #[test]
    fn malformed_headers_cannot_authorize_out_of_range_reads() {
        let packed = pack(&[asset(0, AssetKind::Track, vec![1; 32])]).unwrap();
        let original = packed.volumes[0][..HEADER_BYTES].to_vec();
        assert_eq!(get16(&original, 8), 2);
        assert!(parse_header(&original[..300]).is_err());
        let mut legacy = original.clone();
        put16(&mut legacy, 8, 1);
        resign(&mut legacy);
        assert!(parse_header(&legacy).unwrap_err().0.contains("version"));
        let mut bad = original.clone();
        put16(&mut bad, 8, 99);
        resign(&mut bad);
        assert!(parse_header(&bad).unwrap_err().0.contains("version"));
        let mut bad = original.clone();
        put32(&mut bad, DIRECTORY_OFFSET + 8, u32::MAX);
        resign(&mut bad);
        assert!(parse_header(&bad).is_err());
        let mut bad = original.clone();
        bad[DIRECTORY_OFFSET + 3] = 4;
        resign(&mut bad);
        assert!(parse_header(&bad).is_err());
        let mut bad = original;
        bad[12] = 3;
        assert!(parse_header(&bad).unwrap_err().0.contains("checksum"));
    }
    #[test]
    fn refuses_duplicate_empty_and_oversize_assets() {
        assert!(pack(&[]).is_err());
        assert!(pack(&[asset(1, AssetKind::Track, vec![])]).is_err());
        assert!(
            pack(&[
                asset(1, AssetKind::Track, vec![1]),
                asset(1, AssetKind::Track, vec![2])
            ])
            .is_err()
        );
        assert!(pack(&[asset(1, AssetKind::Track, vec![0; TOTAL_PAYLOAD_BYTES + 1])]).is_err());
    }
    #[test]
    fn asset_crosses_volume_boundary_and_skips_replicated_header() {
        let bytes: Vec<_> = (0..PAYLOAD_BYTES + 777).map(|i| (i % 251) as u8).collect();
        let packed = pack(&[asset(0, AssetKind::Track, bytes.clone())]).unwrap();
        assert_eq!(packed.volumes.len(), 2);
        assert_eq!(packed.entries[0].length as usize, bytes.len());
        assert_eq!(&packed.volumes[0][HEADER_BYTES..], &bytes[..PAYLOAD_BYTES]);
        assert_eq!(
            &packed.volumes[1][HEADER_BYTES..HEADER_BYTES + 777],
            &bytes[PAYLOAD_BYTES..]
        );
        validate_set(&packed.volumes.iter().map(Vec::as_slice).collect::<Vec<_>>()).unwrap();
        assert!(validate_set(&[packed.volumes[0].as_slice()]).unwrap_err().0.contains("volume count"));
    }
    #[test]
    fn volume_count_follows_content_and_is_recorded_in_every_header() {
        let one = pack(&[asset(0, AssetKind::Track, vec![7; PAYLOAD_BYTES])]).unwrap();
        assert_eq!((one.volumes.len(), one.volumes[0][13]), (1, 1));
        let two = pack(&[asset(0, AssetKind::Track, vec![7; PAYLOAD_BYTES + 1])]).unwrap();
        assert_eq!((two.volumes.len(), two.volumes[0][13], two.volumes[1][13]), (2, 2, 2));
        assert_eq!(parse_header(&two.volumes[1]).unwrap().volume_count, 2);
        // A lone header cannot know the set size, so a consistent count of 3 parses;
        // the set check rejects it. Out-of-range counts and indexes fail at once.
        let mut bad = two.volumes[0].clone();
        bad[13] = 3;
        resign(&mut bad);
        assert_eq!(parse_header(&bad).unwrap().volume_count, 3);
        assert!(validate_set(&[bad.as_slice(), two.volumes[1].as_slice()]).unwrap_err().0.contains("volume count"));
        bad[13] = 5;
        resign(&mut bad);
        assert!(parse_header(&bad).unwrap_err().0.contains("index/count"));
        bad[13] = 2;
        bad[12] = 2;
        resign(&mut bad);
        assert!(parse_header(&bad).unwrap_err().0.contains("index/count"));
    }
}
