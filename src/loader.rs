//! Bounded, nonblocking reference loader for one active and one pending selection.
//!
//! This module performs no I/O. A platform reads at most the requested bytes and
//! may DMA directly into the requested REU range, then calls `accept_read` with
//! readback bytes for CRC validation. Superseded DMA retains its pinned allocation
//! until a completion/error acknowledges that writes have stopped. New requests
//! queue during this drain and cannot issue reads. Commit requires primed audio.

use crate::volume::{
    ALIGNMENT, AssetEntry, AssetKind, Crc32, HEADER_BYTES, PAYLOAD_BYTES, TOTAL_PAYLOAD_BYTES,
    MAX_VOLUME_COUNT, VOLUME_BYTES,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub address: u32,
    pub length: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub token: u64,
    pub track_id: u16,
    pub panorama_id: u16,
    pub allocation: Allocation,
    pub track_address: u32,
    pub panorama_address: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Reading,
    Priming,
    Ready,
    Draining,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadRequest {
    pub selection_token: u64,
    pub chunk_token: u64,
    pub volume: u8,
    pub offset: u32,
    pub length: u32,
    pub destination: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// DMA completed and bytes validate, or buffered I/O may now copy them here.
    Accepted { destination: u32, length: u32 },
    /// Superseded DMA is now retired; buffered I/O must discard these bytes.
    Stale,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoaderError {
    InvalidArena,
    InvalidAsset,
    NoSpace,
    TokenExhausted,
    MissingVolume(u8),
    ReadFailed(String),
    ShortRead { expected: u32, actual: usize },
    ChecksumMismatch { kind: AssetKind, id: u16 },
    NotPriming,
    NotReady,
    StaleSelection,
}
impl fmt::Display for LoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LoaderError {}

struct Pending {
    selection: Selection,
    assets: [AssetEntry; 2],
    asset_index: usize,
    received: u32,
    crc: Crc32,
    phase: Phase,
    in_flight: Option<ReadRequest>,
}

pub struct Loader {
    arena: Allocation,
    active: Option<Selection>,
    pending: Option<Pending>,
    draining: Option<(ReadRequest, Allocation)>,
    queued: Option<(u64, AssetEntry, AssetEntry)>,
    next_token: u64,
    next_chunk: u64,
    last_error: Option<LoaderError>,
}

impl Loader {
    /// The arena excludes the resident program, decoder rings, and reserved REU.
    pub fn new(address: u32, length: u32) -> Result<Self, LoaderError> {
        let end = address
            .checked_add(length)
            .ok_or(LoaderError::InvalidArena)?;
        if length == 0 || !address.is_multiple_of(ALIGNMENT as u32) || end > VOLUME_BYTES as u32 {
            return Err(LoaderError::InvalidArena);
        }
        Ok(Self {
            arena: Allocation { address, length },
            active: None,
            pending: None,
            draining: None,
            queued: None,
            next_token: 1,
            next_chunk: 1,
            last_error: None,
        })
    }

    pub fn active(&self) -> Option<&Selection> {
        self.active.as_ref()
    }
    pub fn pending(&self) -> Option<&Selection> {
        self.pending.as_ref().map(|p| &p.selection)
    }
    pub fn phase(&self) -> Phase {
        if self.draining.is_some() {
            return Phase::Draining;
        }
        self.pending.as_ref().map_or(Phase::Idle, |p| p.phase)
    }
    pub fn last_error(&self) -> Option<&LoaderError> {
        self.last_error.as_ref()
    }

    /// Latest request wins, even if it fails validation or cannot fit. The active
    /// selection stays resident. Any outstanding direct DMA drains before its
    /// allocation is reused, while only the latest requested selection is queued.
    pub fn request(&mut self, track: AssetEntry, panorama: AssetEntry) -> Result<u64, LoaderError> {
        self.retire_pending();
        self.queued = None;
        self.last_error = None;
        let token = self.next_token;
        self.next_token = token.checked_add(1).ok_or(LoaderError::TokenExhausted)?;
        match self.prepare(token, track, panorama) {
            Ok(pending) => {
                if self.draining.is_some() {
                    let [track, panorama] = pending.assets;
                    self.queued = Some((token, track, panorama));
                } else {
                    self.pending = Some(pending);
                }
                Ok(token)
            }
            Err(error) => {
                self.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn draining_allocation(&self) -> Option<Allocation> {
        self.draining.map(|(_, allocation)| allocation)
    }
    pub fn queued_token(&self) -> Option<u64> {
        self.queued.as_ref().map(|(token, _, _)| *token)
    }

    fn retire_pending(&mut self) {
        if let Some(pending) = self.pending.take()
            && let Some(read) = pending.in_flight
        {
            self.draining = Some((read, pending.selection.allocation));
        }
    }

    fn acknowledge_drained(&mut self, read: ReadRequest) -> bool {
        if self
            .draining
            .as_ref()
            .is_none_or(|(outstanding, _)| *outstanding != read)
        {
            return false;
        }
        self.draining = None;
        if let Some((token, track, art)) = self.queued.take() {
            match self.prepare(token, track, art) {
                Ok(pending) => self.pending = Some(pending),
                Err(error) => self.last_error = Some(error),
            }
        }
        true
    }

    fn prepare(
        &self,
        token: u64,
        track: AssetEntry,
        panorama: AssetEntry,
    ) -> Result<Pending, LoaderError> {
        if track.kind != AssetKind::Track || panorama.kind != AssetKind::Panorama {
            return Err(LoaderError::InvalidAsset);
        }
        for entry in [&track, &panorama] {
            let start = entry.volume as u64 * PAYLOAD_BYTES as u64
                + entry.offset.saturating_sub(HEADER_BYTES as u32) as u64;
            let end = start + entry.length as u64;
            if entry.length == 0
                || entry.offset < HEADER_BYTES as u32
                || !entry.offset.is_multiple_of(ALIGNMENT as u32)
                || entry.offset >= VOLUME_BYTES as u32
                || end > TOTAL_PAYLOAD_BYTES as u64
                || entry.volume as usize >= MAX_VOLUME_COUNT
            {
                return Err(LoaderError::InvalidAsset);
            }
        }
        let art_offset = align(track.length).ok_or(LoaderError::NoSpace)?;
        let length = align(
            art_offset
                .checked_add(panorama.length)
                .ok_or(LoaderError::NoSpace)?,
        )
        .ok_or(LoaderError::NoSpace)?;
        let allocation = self.allocate(length).ok_or(LoaderError::NoSpace)?;
        let selection = Selection {
            token,
            track_id: track.id,
            panorama_id: panorama.id,
            track_address: allocation.address,
            panorama_address: allocation.address + art_offset,
            allocation,
        };
        Ok(Pending {
            selection,
            assets: [track, panorama],
            asset_index: 0,
            received: 0,
            crc: Crc32::new(),
            phase: Phase::Reading,
            in_flight: None,
        })
    }

    fn allocate(&self, length: u32) -> Option<Allocation> {
        let end = self.arena.address + self.arena.length;
        let address = if let Some(active) = &self.active {
            let a = active.allocation;
            if length <= a.address - self.arena.address {
                self.arena.address
            } else {
                let candidate = a.address.checked_add(a.length)?;
                if length > end.checked_sub(candidate)? {
                    return None;
                }
                candidate
            }
        } else {
            if length > self.arena.length {
                return None;
            }
            self.arena.address
        };
        Some(Allocation { address, length })
    }

    /// At most one outstanding chunk; `max_bytes` is the caller's frame budget.
    /// Zero means no work. This never waits for a file, volume, or storage device.
    pub fn next_read(&mut self, max_bytes: u32) -> Option<ReadRequest> {
        if max_bytes == 0 || self.draining.is_some() {
            return None;
        }
        let pending = self.pending.as_mut()?;
        if pending.phase != Phase::Reading || pending.in_flight.is_some() {
            return None;
        }
        let next_chunk = self.next_chunk.checked_add(1)?;
        let entry = &pending.assets[pending.asset_index];
        let position = entry.volume as u32 * PAYLOAD_BYTES as u32 + entry.offset
            - HEADER_BYTES as u32
            + pending.received;
        let file_offset = HEADER_BYTES as u32 + position % PAYLOAD_BYTES as u32;
        let base = if pending.asset_index == 0 {
            pending.selection.track_address
        } else {
            pending.selection.panorama_address
        };
        let request = ReadRequest {
            selection_token: pending.selection.token,
            chunk_token: self.next_chunk,
            volume: (position / PAYLOAD_BYTES as u32) as u8,
            offset: file_offset,
            length: max_bytes
                .min(entry.length - pending.received)
                .min(VOLUME_BYTES as u32 - file_offset),
            destination: base + pending.received,
        };
        self.next_chunk = next_chunk;
        pending.in_flight = Some(request);
        Some(request)
    }

    fn is_current(&self, read: ReadRequest) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| p.in_flight == Some(read))
    }

    /// Input length must exactly match the bounded read request. Short reads and
    /// checksum failures discard only the pending selection, keeping active audio.
    pub fn accept_read(
        &mut self,
        read: ReadRequest,
        bytes: &[u8],
    ) -> Result<ReadOutcome, LoaderError> {
        if self.acknowledge_drained(read) {
            return Ok(ReadOutcome::Stale);
        }
        if !self.is_current(read) {
            return Ok(ReadOutcome::Stale);
        }
        if bytes.len() != read.length as usize {
            return self.abort(LoaderError::ShortRead {
                expected: read.length,
                actual: bytes.len(),
            });
        }
        let pending = self.pending.as_mut().unwrap();
        pending.crc.update(bytes);
        pending.received += read.length;
        pending.in_flight = None;
        let entry = &pending.assets[pending.asset_index];
        if pending.received == entry.length {
            if pending.crc.finish() != entry.crc32 {
                let error = LoaderError::ChecksumMismatch {
                    kind: entry.kind,
                    id: entry.id,
                };
                return self.abort(error);
            }
            pending.asset_index += 1;
            pending.received = 0;
            pending.crc = Crc32::new();
            if pending.asset_index == 2 {
                pending.phase = Phase::Priming;
            }
        }
        Ok(ReadOutcome::Accepted {
            destination: read.destination,
            length: read.length,
        })
    }

    /// Call only after DMA has stopped. Superseded errors retire their pinned
    /// allocation and return false without failing the newly queued selection.
    pub fn fail_read(&mut self, read: ReadRequest, error: LoaderError) -> bool {
        if self.acknowledge_drained(read) {
            return false;
        }
        if !self.is_current(read) {
            return false;
        }
        self.pending = None;
        self.last_error = Some(error);
        true
    }

    fn abort<T>(&mut self, error: LoaderError) -> Result<T, LoaderError> {
        self.pending = None;
        self.last_error = Some(error.clone());
        Err(error)
    }

    /// The platform calls this only after decoder startup/ring prefill succeeds.
    pub fn mark_primed(&mut self, token: u64) -> Result<(), LoaderError> {
        let pending = self.pending.as_mut().ok_or(LoaderError::StaleSelection)?;
        if pending.selection.token != token {
            return Err(LoaderError::StaleSelection);
        }
        if pending.phase != Phase::Priming {
            return Err(LoaderError::NotPriming);
        }
        pending.phase = Phase::Ready;
        Ok(())
    }

    /// Atomically exchanges resident ownership; the caller switches audio and art
    /// together at its safe boundary. Before this call, active owns the old data.
    pub fn commit(&mut self, token: u64) -> Result<&Selection, LoaderError> {
        let pending = self.pending.as_ref().ok_or(LoaderError::StaleSelection)?;
        if pending.selection.token != token {
            return Err(LoaderError::StaleSelection);
        }
        if pending.phase != Phase::Ready {
            return Err(LoaderError::NotReady);
        }
        self.active = Some(self.pending.take().unwrap().selection);
        self.last_error = None;
        Ok(self.active.as_ref().unwrap())
    }

    /// Priming can fail after all file reads have completed.
    pub fn fail_pending(&mut self, token: u64, error: LoaderError) -> bool {
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.selection.token != token)
        {
            return false;
        }
        self.retire_pending();
        self.last_error = Some(error);
        true
    }

    pub fn cancel_pending(&mut self) {
        self.retire_pending();
        self.queued = None;
        self.last_error = None;
    }
}

fn align(value: u32) -> Option<u32> {
    Some(value.checked_add(ALIGNMENT as u32 - 1)? & !(ALIGNMENT as u32 - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::crc32;
    fn pair(id: u16, track: &[u8], art: &[u8]) -> (AssetEntry, AssetEntry) {
        let entry = |kind, bytes: &[u8], offset| AssetEntry {
            id,
            kind,
            volume: 0,
            offset,
            length: bytes.len() as u32,
            crc32: crc32(bytes),
            metadata: [0; 4],
            name: format!("asset-{id}"),
        };
        (
            entry(AssetKind::Track, track, 4096),
            entry(AssetKind::Panorama, art, 8192),
        )
    }
    fn request(loader: &mut Loader, id: u16) -> u64 {
        let (track, art) = pair(id, &[1; 700], &[2; 300]);
        loader.request(track, art).unwrap()
    }
    fn finish_reads(loader: &mut Loader) {
        while let Some(read) = loader.next_read(128) {
            let value = if read.offset < 8192 { 1 } else { 2 };
            assert_eq!(
                loader
                    .accept_read(read, &vec![value; read.length as usize])
                    .unwrap(),
                ReadOutcome::Accepted {
                    destination: read.destination,
                    length: read.length
                }
            );
        }
        assert_eq!(loader.phase(), Phase::Priming);
    }
    fn activate(loader: &mut Loader, id: u16) -> u64 {
        let token = request(loader, id);
        finish_reads(loader);
        loader.mark_primed(token).unwrap();
        loader.commit(token).unwrap();
        token
    }
    #[test]
    fn bounded_reads_require_complete_assets_and_audio_priming() {
        let mut loader = Loader::new(0x10000, 0x10000).unwrap();
        let token = request(&mut loader, 0);
        assert_eq!(loader.commit(token), Err(LoaderError::NotReady));
        assert_eq!(loader.mark_primed(token), Err(LoaderError::NotPriming));
        assert!(loader.next_read(0).is_none());
        let read = loader.next_read(128).unwrap();
        assert_eq!(read.length, 128);
        assert!(loader.next_read(128).is_none());
        loader.accept_read(read, &[1; 128]).unwrap();
        finish_reads(&mut loader);
        assert!(loader.active().is_none());
        assert_eq!(loader.commit(token), Err(LoaderError::NotReady));
        loader.mark_primed(token).unwrap();
        loader.commit(token).unwrap();
        assert_eq!(loader.active().unwrap().track_id, 0);
        assert_eq!(loader.phase(), Phase::Idle);
    }
    #[test]
    fn latest_request_wins_and_stale_io_never_authorizes_writes() {
        let mut loader = Loader::new(0, 0x10000).unwrap();
        activate(&mut loader, 1);
        let old = request(&mut loader, 2);
        let read = loader.next_read(100).unwrap();
        let pinned = loader.pending().unwrap().allocation;
        let newest = request(&mut loader, 3);
        assert_eq!(loader.phase(), Phase::Draining);
        assert_eq!(loader.draining_allocation(), Some(pinned));
        assert_eq!(loader.queued_token(), Some(newest));
        assert!(loader.next_read(128).is_none());
        assert!(loader.pending().is_none());
        assert_eq!(
            loader.accept_read(read, &[9; 100]).unwrap(),
            ReadOutcome::Stale
        );
        assert!(!loader.fail_read(read, LoaderError::MissingVolume(0)));
        assert_eq!(loader.mark_primed(old), Err(LoaderError::StaleSelection));
        assert_eq!(loader.active().unwrap().track_id, 1);
        finish_reads(&mut loader);
        loader.mark_primed(newest).unwrap();
        loader.commit(newest).unwrap();
        assert_eq!(loader.active().unwrap().track_id, 3);
    }
    #[test]
    fn file_reads_split_at_volume_boundary_and_skip_header() {
        let mut loader = Loader::new(0, 65536).unwrap();
        let (mut track, art) = pair(0, &[1; 700], &[2; 300]);
        track.offset = VOLUME_BYTES as u32 - 256;
        let token = loader.request(track, art).unwrap();
        let first = loader.next_read(1024).unwrap();
        assert_eq!(
            (first.volume, first.offset, first.length),
            (0, VOLUME_BYTES as u32 - 256, 256)
        );
        loader.accept_read(first, &[1; 256]).unwrap();
        let second = loader.next_read(1024).unwrap();
        assert_eq!(
            (second.volume, second.offset, second.length),
            (1, HEADER_BYTES as u32, 444)
        );
        loader.accept_read(second, &[1; 444]).unwrap();
        let art = loader.next_read(1024).unwrap();
        loader.accept_read(art, &[2; 300]).unwrap();
        loader.mark_primed(token).unwrap();
        loader.commit(token).unwrap();
    }
    #[test]
    fn cancel_and_reselect_still_wait_for_direct_dma_to_stop() {
        let mut loader = Loader::new(0, 65536).unwrap();
        activate(&mut loader, 1);
        request(&mut loader, 2);
        let read = loader.next_read(128).unwrap();
        loader.cancel_pending();
        assert_eq!(loader.phase(), Phase::Draining);
        request(&mut loader, 3);
        let newest = request(&mut loader, 4);
        assert!(loader.next_read(128).is_none());
        assert!(!loader.fail_read(read, LoaderError::ReadFailed("canceled DMA stopped".into())));
        assert_eq!(loader.phase(), Phase::Reading);
        assert_eq!(loader.pending().unwrap().token, newest);
        assert_eq!(loader.active().unwrap().track_id, 1);
        finish_reads(&mut loader);
        loader.mark_primed(newest).unwrap();
        loader.commit(newest).unwrap();
        assert_eq!(loader.active().unwrap().track_id, 4);
    }
    #[test]
    fn missing_volume_short_read_crc_and_priming_failures_preserve_active() {
        let mut loader = Loader::new(0, 0x10000).unwrap();
        activate(&mut loader, 1);
        let active = loader.active().unwrap().clone();
        request(&mut loader, 2);
        let read = loader.next_read(100).unwrap();
        assert!(loader.fail_read(read, LoaderError::MissingVolume(0)));
        assert_eq!(loader.active(), Some(&active));
        request(&mut loader, 2);
        let read = loader.next_read(100).unwrap();
        assert!(matches!(
            loader.accept_read(read, &[1; 99]),
            Err(LoaderError::ShortRead { .. })
        ));
        assert_eq!(loader.active(), Some(&active));
        request(&mut loader, 2);
        let read = loader.next_read(1000).unwrap();
        assert!(matches!(
            loader.accept_read(read, &vec![3; read.length as usize]),
            Err(LoaderError::ChecksumMismatch { .. })
        ));
        assert_eq!(loader.active(), Some(&active));
        let token = request(&mut loader, 2);
        finish_reads(&mut loader);
        assert!(loader.fail_pending(
            token,
            LoaderError::ReadFailed("decoder rejected stream".into())
        ));
        assert_eq!(loader.active(), Some(&active));
    }
    #[test]
    fn allocator_keeps_active_and_pending_disjoint_and_reuses_released_memory() {
        let mut loader = Loader::new(0x1000, 0x1000).unwrap();
        activate(&mut loader, 1);
        let first = loader.active().unwrap().allocation;
        let next = request(&mut loader, 2);
        let pending = loader.pending().unwrap().allocation;
        assert!(pending.address >= first.address + first.length);
        finish_reads(&mut loader);
        loader.mark_primed(next).unwrap();
        loader.commit(next).unwrap();
        request(&mut loader, 3);
        assert_eq!(loader.pending().unwrap().allocation.address, first.address);
        assert_eq!(loader.active().unwrap().allocation, pending);
    }
    #[test]
    fn no_space_never_evicts_active_and_invalid_new_request_cancels_old_pending() {
        let mut loader = Loader::new(0, 1536).unwrap();
        activate(&mut loader, 1);
        let (track, art) = pair(2, &[1; 700], &[2; 300]);
        assert_eq!(loader.request(track, art), Err(LoaderError::NoSpace));
        assert_eq!(loader.active().unwrap().track_id, 1);
        let mut loader = Loader::new(0, 65536).unwrap();
        request(&mut loader, 1);
        let (mut track, art) = pair(2, &[1; 700], &[2; 300]);
        track.length = u32::MAX;
        assert_eq!(loader.request(track, art), Err(LoaderError::InvalidAsset));
        assert!(loader.pending().is_none());
    }
}
