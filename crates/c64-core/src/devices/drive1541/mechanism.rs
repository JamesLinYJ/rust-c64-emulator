// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 rotating media mechanism
//
//   File:       devices/drive1541/mechanism.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt::{self, Write as _};

use crate::media::d64::{D64DiskImage, D64ImageError};
use crate::media::g64::{G64DiskImage, G64ImageError, G64SpeedMap, G64SpeedZone};

use super::d64_gcr::{
    D64_GCR_RAW_TRACK_SIZE, D64GcrError, build_d64_gcr_track, d64_speed_zone_for_track,
    decode_d64_gcr_track,
};
use super::gcr::{
    Drive1541GcrCircuit, Drive1541GcrError, Drive1541GcrSignals, Drive1541SpeedZone,
    GCR_REFERENCE_TICKS_PER_CPU_CYCLE,
};

pub const DRIVE_1541_MINIMUM_HALF_TRACK: u8 = 2;
pub const DRIVE_1541_MAXIMUM_HALF_TRACK: u8 = 85;
pub const DRIVE_1541_INITIAL_HALF_TRACK: u8 = 36;
pub const DRIVE_1541_DISK_INSERTION_CYCLES: u64 = 1_800_000;
pub const DRIVE_1541_DISK_REMOVAL_CYCLES: u64 = 600_000;
pub const DRIVE_1541_DISK_REPLACEMENT_GAP_CYCLES: u64 = 1_200_000;

const BITS_PER_BYTE: usize = 8;
const RAW_TRACK_SLOT_COUNT: usize = 84;
const REFERENCE_TICKS_PER_REVOLUTION: u64 = 3_200_000;
const STEPPER_PHASE_MASK: u8 = 0x03;
const CONTROL_MOTOR_ON: u8 = 1 << 0;
const CONTROL_LED_ON: u8 = 1 << 1;
const CONTROL_READING: u8 = 1 << 2;
const CONTROL_BYTE_READY_ENABLED: u8 = 1 << 3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum Drive1541StepperPhase {
    #[default]
    Phase0 = 0,
    Phase1 = 1,
    Phase2 = 2,
    Phase3 = 3,
}

impl Drive1541StepperPhase {
    pub const ALL: [Self; 4] = [Self::Phase0, Self::Phase1, Self::Phase2, Self::Phase3];

    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for Drive1541StepperPhase {
    type Error = Drive1541MechanismError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Phase0),
            1 => Ok(Self::Phase1),
            2 => Ok(Self::Phase2),
            3 => Ok(Self::Phase3),
            _ => Err(Drive1541MechanismError::InvalidStepperPhase(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Drive1541ControlState {
    pub led_on: bool,
    pub motor_on: bool,
    pub speed_zone: Drive1541SpeedZone,
    pub stepper_phase: Drive1541StepperPhase,
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum Drive1541DiskImage {
    D64(D64DiskImage),
    G64(G64DiskImage),
}

impl Drive1541DiskImage {
    pub const fn write_protected(&self) -> bool {
        match self {
            Self::D64(image) => image.write_protected(),
            Self::G64(image) => image.write_protected(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Drive1541D64CommitFailure {
    pub half_track: u8,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Drive1541D64CommitReport {
    pub committed_half_tracks: Vec<u8>,
    pub failures: Vec<Drive1541D64CommitFailure>,
    pub remaining_dirty_half_tracks: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Drive1541MechanismError {
    DiskAlreadyMounted,
    Empty,
    UncommittedD64Writes,
    RawTrackDiscardRequiresD64,
    SectorProjectionRequiresD64,
    InvalidHalfTrack(u8),
    InvalidStepperPhase(u8),
    AngularOffsetOutOfBounds {
        half_track: u8,
        bit_offset: usize,
        track_bits: usize,
    },
    ArithmeticOverflow,
    ValidatedSectorMissing {
        track: u8,
        sector: u8,
    },
    D64(D64ImageError),
    D64Gcr(D64GcrError),
    G64(G64ImageError),
    Gcr(Drive1541GcrError),
}

impl fmt::Display for Drive1541MechanismError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiskAlreadyMounted => {
                write!(formatter, "a disk is already mounted in the 1541 mechanism")
            }
            Self::Empty => write!(formatter, "the 1541 mechanism contains no disk"),
            Self::UncommittedD64Writes => write!(
                formatter,
                "cannot eject a D64 with uncommitted raw-track writes"
            ),
            Self::RawTrackDiscardRequiresD64 => write!(
                formatter,
                "raw-track discard is only defined for a mounted D64 image"
            ),
            Self::SectorProjectionRequiresD64 => write!(
                formatter,
                "sector projection is only defined for a mounted D64 image"
            ),
            Self::InvalidHalfTrack(half_track) => write!(
                formatter,
                "1541 half-track {half_track} is outside {DRIVE_1541_MINIMUM_HALF_TRACK}..={DRIVE_1541_MAXIMUM_HALF_TRACK}"
            ),
            Self::InvalidStepperPhase(phase) => {
                write!(formatter, "1541 stepper phase {phase} is outside 0..=3")
            }
            Self::AngularOffsetOutOfBounds {
                half_track,
                bit_offset,
                track_bits,
            } => write!(
                formatter,
                "1541 half-track {half_track} bit offset {bit_offset} exceeds {track_bits} bits"
            ),
            Self::ArithmeticOverflow => {
                write!(
                    formatter,
                    "1541 mechanism interval exceeds integer timing range"
                )
            }
            Self::ValidatedSectorMissing { track, sector } => write!(
                formatter,
                "validated D64 GCR track {track} is missing sector {sector}"
            ),
            Self::D64(error) => error.fmt(formatter),
            Self::D64Gcr(error) => error.fmt(formatter),
            Self::G64(error) => error.fmt(formatter),
            Self::Gcr(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Drive1541MechanismError {}

impl From<D64ImageError> for Drive1541MechanismError {
    fn from(error: D64ImageError) -> Self {
        Self::D64(error)
    }
}

impl From<D64GcrError> for Drive1541MechanismError {
    fn from(error: D64GcrError) -> Self {
        Self::D64Gcr(error)
    }
}

impl From<G64ImageError> for Drive1541MechanismError {
    fn from(error: G64ImageError) -> Self {
        Self::G64(error)
    }
}

impl From<Drive1541GcrError> for Drive1541MechanismError {
    fn from(error: Drive1541GcrError) -> Self {
        Self::Gcr(error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
struct ByteReadyState {
    asserted: bool,
    transition_sequence: u64,
    edge_sequence: u64,
    last_edge_data: u8,
}

impl ByteReadyState {
    fn set_asserted(&mut self, asserted: bool) {
        if self.asserted == asserted {
            return;
        }
        self.asserted = asserted;
        self.transition_sequence = self.transition_sequence.wrapping_add(1);
    }

    fn signal_edge(&mut self, data_byte: u8) {
        self.edge_sequence = self.edge_sequence.wrapping_add(1);
        self.last_edge_data = data_byte;
        self.set_asserted(true);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
struct MechanismMediaState {
    disk: Option<Drive1541DiskImage>,
    raw_tracks: [Option<Vec<u8>>; RAW_TRACK_SLOT_COUNT],
    dirty_raw_tracks: [bool; RAW_TRACK_SLOT_COUNT],
}

impl Default for MechanismMediaState {
    fn default() -> Self {
        Self {
            disk: None,
            raw_tracks: std::array::from_fn(|_| None),
            dirty_raw_tracks: [false; RAW_TRACK_SLOT_COUNT],
        }
    }
}

impl MechanismMediaState {
    fn clear_raw_tracks(&mut self) {
        for track in &mut self.raw_tracks {
            *track = None;
        }
        self.dirty_raw_tracks.fill(false);
    }

    fn dirty_half_tracks(&self) -> Vec<u8> {
        (DRIVE_1541_MINIMUM_HALF_TRACK..=DRIVE_1541_MAXIMUM_HALF_TRACK)
            .zip(self.dirty_raw_tracks.iter().copied())
            .filter(|(_, dirty)| *dirty)
            .map(|(half_track, _)| half_track)
            .collect()
    }

    fn track_size_bytes(&self, half_track: u8) -> Result<usize, Drive1541MechanismError> {
        require_half_track(half_track)?;
        if let Some(Drive1541DiskImage::G64(image)) = &self.disk
            && let Some(track) = image.half_track(half_track)?
        {
            return Ok(track.bytes().len());
        }
        let track = half_track / 2;
        let speed_zone = d64_speed_zone_for_track(track)?;
        Ok(D64_GCR_RAW_TRACK_SIZE[usize::from(speed_zone.get())])
    }

    fn track_size_bits(&self, half_track: u8) -> Result<usize, Drive1541MechanismError> {
        self.track_size_bytes(half_track)?
            .checked_mul(BITS_PER_BYTE)
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)
    }

    fn raw_track_mut(&mut self, half_track: u8) -> Result<&mut Vec<u8>, Drive1541MechanismError> {
        let index = half_track_index(half_track)?;
        if self.disk.is_none() {
            return Err(Drive1541MechanismError::Empty);
        }
        if self.raw_tracks[index].is_none() {
            let track_number = half_track / 2;
            let bytes = match self.disk.as_ref().ok_or(Drive1541MechanismError::Empty)? {
                Drive1541DiskImage::G64(image) => {
                    if let Some(track) = image.half_track(half_track)? {
                        track.bytes().to_vec()
                    } else {
                        vec![0; self.track_size_bytes(half_track)?]
                    }
                }
                Drive1541DiskImage::D64(image) => {
                    if half_track.is_multiple_of(2) && track_number <= image.track_count() {
                        build_d64_gcr_track(image, track_number)?.bytes
                    } else {
                        vec![0; self.track_size_bytes(half_track)?]
                    }
                }
            };
            self.raw_tracks[index] = Some(bytes);
        }
        self.raw_tracks[index]
            .as_mut()
            .ok_or(Drive1541MechanismError::Empty)
    }

    fn read_current_bit(
        &mut self,
        half_track: u8,
        angular_bit_offset: usize,
    ) -> Result<bool, Drive1541MechanismError> {
        if self.disk.is_none() {
            return Ok(false);
        }
        let track = self.raw_track_mut(half_track)?;
        let track_bits = track
            .len()
            .checked_mul(BITS_PER_BYTE)
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
        let byte_index = angular_bit_offset / BITS_PER_BYTE;
        let bit_in_byte = 7 - angular_bit_offset % BITS_PER_BYTE;
        let value = track.get(byte_index).copied().ok_or(
            Drive1541MechanismError::AngularOffsetOutOfBounds {
                half_track,
                bit_offset: angular_bit_offset,
                track_bits,
            },
        )?;
        Ok(value & (1 << bit_in_byte) != 0)
    }

    fn write_current_bit(
        &mut self,
        half_track: u8,
        angular_bit_offset: usize,
        speed_zone: Drive1541SpeedZone,
        high: bool,
    ) -> Result<(), Drive1541MechanismError> {
        let Some(disk) = self.disk.as_ref() else {
            return Ok(());
        };
        if disk.write_protected() {
            return Ok(());
        }

        let track_index = half_track_index(half_track)?;
        let g64_track_was_absent = matches!(
            self.disk.as_ref(),
            Some(Drive1541DiskImage::G64(image)) if !image.has_half_track(half_track)?
        );
        let (byte_index, next, absent_track_bytes) = {
            let track = self.raw_track_mut(half_track)?;
            let track_bits = track
                .len()
                .checked_mul(BITS_PER_BYTE)
                .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
            let byte_index = angular_bit_offset / BITS_PER_BYTE;
            let bit_in_byte = 7 - angular_bit_offset % BITS_PER_BYTE;
            let byte = track.get_mut(byte_index).ok_or(
                Drive1541MechanismError::AngularOffsetOutOfBounds {
                    half_track,
                    bit_offset: angular_bit_offset,
                    track_bits,
                },
            )?;
            let mask = 1 << bit_in_byte;
            *byte = if high { *byte | mask } else { *byte & !mask };
            let next = *byte;
            let snapshot = g64_track_was_absent.then(|| track.clone());
            (byte_index, next, snapshot)
        };

        match self.disk.as_mut().ok_or(Drive1541MechanismError::Empty)? {
            Drive1541DiskImage::G64(image) => {
                let g64_speed_zone = to_g64_speed_zone(speed_zone);
                if let Some(track) = absent_track_bytes {
                    image.set_half_track(
                        half_track,
                        &track,
                        Some(G64SpeedMap::Constant(g64_speed_zone)),
                    )?;
                }
                image.write_half_track_byte(half_track, byte_index, next, Some(g64_speed_zone))?;
            }
            Drive1541DiskImage::D64(_) => {
                self.dirty_raw_tracks[track_index] = true;
            }
        }
        Ok(())
    }
}

struct MechanismGcrSignals<'a> {
    media: &'a mut MechanismMediaState,
    byte_ready: &'a mut ByteReadyState,
    angular_bit_offset: &'a mut usize,
    media_rotation_accumulator: &'a mut u64,
    current_half_track: u8,
    selected_speed_zone: Drive1541SpeedZone,
    error: Option<Drive1541MechanismError>,
}

impl MechanismGcrSignals<'_> {
    fn write_flux_bit(&mut self, high: bool) -> Result<(), Drive1541MechanismError> {
        if self.media.disk.is_none() {
            return Ok(());
        }
        self.media.write_current_bit(
            self.current_half_track,
            *self.angular_bit_offset,
            self.selected_speed_zone,
            high,
        )?;
        let track_bits = self.media.track_size_bits(self.current_half_track)?;
        *self.angular_bit_offset = (*self.angular_bit_offset + 1) % track_bits;
        *self.media_rotation_accumulator = u64::try_from(
            track_bits
                .checked_mul(2)
                .ok_or(Drive1541MechanismError::ArithmeticOverflow)?,
        )
        .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
        Ok(())
    }
}

impl Drive1541GcrSignals for MechanismGcrSignals<'_> {
    fn signal_byte_ready(&mut self, data_byte: u8) {
        self.byte_ready.signal_edge(data_byte);
    }

    fn write_flux_bit(&mut self, high: bool) {
        if self.error.is_some() {
            return;
        }
        self.error = MechanismGcrSignals::write_flux_bit(self, high).err();
    }
}

/// Owns the 1541 spindle, head, media cache and read/write separator. It has no
/// knowledge of VIA register addresses or IEC protocol state.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Drive1541Mechanism {
    media: MechanismMediaState,
    gcr_circuit: Drive1541GcrCircuit,
    current_half_track: u8,
    angular_bit_offset: usize,
    media_rotation_accumulator: u64,
    selected_speed_zone: Drive1541SpeedZone,
    control_flags: u8,
    byte_ready: ByteReadyState,
    disk_insertion_cycles_remaining: u64,
    disk_removal_cycles_remaining: u64,
    disk_replacement_gap_cycles_remaining: u64,
}

impl Default for Drive1541Mechanism {
    fn default() -> Self {
        Self::new()
    }
}

impl Drive1541Mechanism {
    pub fn new() -> Self {
        Self {
            media: MechanismMediaState::default(),
            gcr_circuit: Drive1541GcrCircuit::new(),
            current_half_track: DRIVE_1541_INITIAL_HALF_TRACK,
            angular_bit_offset: 0,
            media_rotation_accumulator: 0,
            selected_speed_zone: Drive1541SpeedZone::Zone0,
            control_flags: CONTROL_LED_ON | CONTROL_READING | CONTROL_BYTE_READY_ENABLED,
            byte_ready: ByteReadyState::default(),
            disk_insertion_cycles_remaining: 0,
            disk_removal_cycles_remaining: 0,
            disk_replacement_gap_cycles_remaining: 0,
        }
    }

    pub const fn disk_present(&self) -> bool {
        self.media.disk.is_some()
    }

    pub const fn mounted_disk(&self) -> Option<&Drive1541DiskImage> {
        self.media.disk.as_ref()
    }

    pub fn write_protected(&self) -> bool {
        self.media
            .disk
            .as_ref()
            .is_none_or(Drive1541DiskImage::write_protected)
    }

    pub fn write_protect_sensor_active(&self) -> bool {
        if self.disk_removal_cycles_remaining > 0 {
            return true;
        }
        if self.disk_replacement_gap_cycles_remaining > 0 {
            return false;
        }
        if self.disk_insertion_cycles_remaining > 0 {
            return true;
        }
        self.media
            .disk
            .as_ref()
            .is_some_and(Drive1541DiskImage::write_protected)
    }

    pub const fn current_half_track(&self) -> u8 {
        self.current_half_track
    }

    pub const fn current_whole_track(&self) -> u8 {
        self.current_half_track / 2
    }

    pub const fn angular_bit_offset(&self) -> usize {
        self.angular_bit_offset
    }

    pub const fn selected_speed_zone(&self) -> Drive1541SpeedZone {
        self.selected_speed_zone
    }

    pub const fn motor_on(&self) -> bool {
        self.control_flag(CONTROL_MOTOR_ON)
    }

    pub const fn led_on(&self) -> bool {
        self.control_flag(CONTROL_LED_ON)
    }

    pub const fn reading(&self) -> bool {
        self.control_flag(CONTROL_READING)
    }

    pub const fn byte_ready_enabled(&self) -> bool {
        self.control_flag(CONTROL_BYTE_READY_ENABLED)
    }

    pub const fn byte_ready_asserted(&self) -> bool {
        self.byte_ready.asserted
    }

    pub const fn byte_ready_transition_sequence(&self) -> u64 {
        self.byte_ready.transition_sequence
    }

    pub const fn byte_ready_edge_sequence(&self) -> u64 {
        self.byte_ready.edge_sequence
    }

    pub const fn last_byte_ready_edge_data(&self) -> u8 {
        self.byte_ready.last_edge_data
    }

    pub const fn data_byte(&self) -> u8 {
        self.gcr_circuit.data_byte()
    }

    pub const fn write_data_byte(&self) -> u8 {
        self.gcr_circuit.write_data_byte()
    }

    pub const fn sync_found(&self) -> bool {
        self.gcr_circuit.sync_found()
    }

    pub fn dirty_half_tracks(&self) -> Vec<u8> {
        self.media.dirty_half_tracks()
    }

    /// Mount media without silently replacing an existing disk.
    ///
    /// # Errors
    ///
    /// Returns `DiskAlreadyMounted` until the current disk is explicitly ejected.
    pub fn mount_disk(&mut self, image: Drive1541DiskImage) -> Result<(), Drive1541MechanismError> {
        if self.media.disk.is_some() {
            return Err(Drive1541MechanismError::DiskAlreadyMounted);
        }
        let replacing_during_removal = self.disk_removal_cycles_remaining > 0;
        self.media.disk = Some(image);
        self.disk_insertion_cycles_remaining = DRIVE_1541_DISK_INSERTION_CYCLES;
        self.disk_replacement_gap_cycles_remaining = if replacing_during_removal {
            DRIVE_1541_DISK_REPLACEMENT_GAP_CYCLES
        } else {
            0
        };
        self.media.clear_raw_tracks();
        self.reset_read_path_at_current_position();
        Ok(())
    }

    /// Eject media without discarding uncommitted D64 raw-track writes.
    ///
    /// # Errors
    ///
    /// Rejects an empty mechanism or a dirty D64.
    pub fn eject_disk(&mut self) -> Result<Drive1541DiskImage, Drive1541MechanismError> {
        let image = self
            .media
            .disk
            .as_ref()
            .ok_or(Drive1541MechanismError::Empty)?;
        if matches!(image, Drive1541DiskImage::D64(_))
            && self.media.dirty_raw_tracks.iter().any(|&dirty| dirty)
        {
            return Err(Drive1541MechanismError::UncommittedD64Writes);
        }
        let image = self
            .media
            .disk
            .take()
            .ok_or(Drive1541MechanismError::Empty)?;
        self.finish_disk_removal();
        Ok(image)
    }

    /// Explicitly discard raw-track state that cannot be represented by D64.
    ///
    /// # Errors
    ///
    /// Rejects empty mechanisms and mounted G64 images.
    pub fn discard_raw_track_writes_and_eject(
        &mut self,
    ) -> Result<D64DiskImage, Drive1541MechanismError> {
        match self.media.disk.as_ref() {
            None => return Err(Drive1541MechanismError::Empty),
            Some(Drive1541DiskImage::G64(_)) => {
                return Err(Drive1541MechanismError::RawTrackDiscardRequiresD64);
            }
            Some(Drive1541DiskImage::D64(_)) => {}
        }
        self.media.dirty_raw_tracks.fill(false);
        let Some(Drive1541DiskImage::D64(image)) = self.media.disk.take() else {
            return Err(Drive1541MechanismError::RawTrackDiscardRequiresD64);
        };
        self.finish_disk_removal();
        Ok(image)
    }

    /// Project complete, checksum-valid full tracks back into D64 sectors.
    ///
    /// # Errors
    ///
    /// Rejects empty mechanisms, G64 media and internal codec failures.
    pub fn commit_raw_track_writes_to_d64(
        &mut self,
    ) -> Result<Drive1541D64CommitReport, Drive1541MechanismError> {
        let track_count = match self.media.disk.as_ref() {
            None => return Err(Drive1541MechanismError::Empty),
            Some(Drive1541DiskImage::G64(_)) => {
                return Err(Drive1541MechanismError::SectorProjectionRequiresD64);
            }
            Some(Drive1541DiskImage::D64(image)) => image.track_count(),
        };
        let mut committed_half_tracks = Vec::new();
        let mut failures = Vec::new();

        for half_track in self.dirty_half_tracks() {
            let track_number = half_track / 2;
            if !half_track.is_multiple_of(2) || track_number > track_count {
                failures.push(Drive1541D64CommitFailure {
                    half_track,
                    reason: String::from(
                        "D64 represents full tracks only, and this half-track has no sector projection.",
                    ),
                });
                continue;
            }

            let raw_track = self.media.raw_track_mut(half_track)?.clone();
            let decoded = decode_d64_gcr_track(&raw_track)?;
            let sectors_on_track = match self.media.disk.as_ref() {
                Some(Drive1541DiskImage::D64(image)) => image.sectors_on_track(track_number)?,
                _ => return Err(Drive1541MechanismError::SectorProjectionRequiresD64),
            };
            let mut by_sector = vec![None; usize::from(sectors_on_track)];
            let mut failure_reason = None;
            for sector in &decoded.sectors {
                if sector.track != track_number {
                    continue;
                }
                let index = usize::from(sector.sector);
                let Some(slot) = by_sector.get_mut(index) else {
                    continue;
                };
                if slot.is_some() {
                    failure_reason = Some(format!(
                        "Track {track_number} contains duplicate sector {}.",
                        sector.sector
                    ));
                    break;
                }
                *slot = Some(sector);
            }
            let decoded_sector_count = by_sector.iter().flatten().count();
            if failure_reason.is_none() && decoded_sector_count != usize::from(sectors_on_track) {
                failure_reason = Some(format!(
                    "Decoded {decoded_sector_count} of {sectors_on_track} required sectors on track {track_number}."
                ));
            }
            let mut disk_id = None;
            if failure_reason.is_none() {
                for sector in by_sector.iter().flatten() {
                    let candidate = (sector.id1, sector.id2);
                    if disk_id.is_some_and(|value| value != candidate) {
                        failure_reason = Some(format!(
                            "Track {track_number} contains inconsistent disk IDs."
                        ));
                        break;
                    }
                    disk_id = Some(candidate);
                }
            }
            if let Some(mut reason) = failure_reason {
                if let Some(issue) = decoded.issues.first() {
                    write!(
                        reason,
                        " First decode issue at bit {}: {}",
                        issue.bit_offset, issue.reason
                    )
                    .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
                }
                failures.push(Drive1541D64CommitFailure { half_track, reason });
                continue;
            }

            let Some(Drive1541DiskImage::D64(image)) = self.media.disk.as_mut() else {
                return Err(Drive1541MechanismError::SectorProjectionRequiresD64);
            };
            for sector_number in 0..sectors_on_track {
                let sector = by_sector[usize::from(sector_number)].ok_or(
                    Drive1541MechanismError::ValidatedSectorMissing {
                        track: track_number,
                        sector: sector_number,
                    },
                )?;
                image.write_sector(track_number, sector_number, &sector.data)?;
            }
            self.media.dirty_raw_tracks[half_track_index(half_track)?] = false;
            committed_half_tracks.push(half_track);
        }

        Ok(Drive1541D64CommitReport {
            committed_half_tracks,
            failures,
            remaining_dirty_half_tracks: self.dirty_half_tracks(),
        })
    }

    /// Apply VIA2 output pins as one coherent board state.
    ///
    /// # Errors
    ///
    /// Returns an error only if media geometry cannot preserve angular position.
    pub fn apply_control_state(
        &mut self,
        state: Drive1541ControlState,
    ) -> Result<(), Drive1541MechanismError> {
        let physical_stepper_position =
            (self.current_half_track - DRIVE_1541_MINIMUM_HALF_TRACK) & STEPPER_PHASE_MASK;
        let phase_delta = state
            .stepper_phase
            .get()
            .wrapping_sub(physical_stepper_position)
            & STEPPER_PHASE_MASK;
        if state.motor_on && matches!(phase_delta, 1 | 3) {
            self.move_head(if phase_delta == 1 { 1 } else { -1 })?;
        }
        self.set_control_flag(CONTROL_MOTOR_ON, state.motor_on);
        self.set_control_flag(CONTROL_LED_ON, state.led_on);
        self.set_speed_zone(state.speed_zone);
        Ok(())
    }

    pub fn set_motor_on(&mut self, motor_on: bool) {
        self.set_control_flag(CONTROL_MOTOR_ON, motor_on);
    }

    pub fn set_led_on(&mut self, led_on: bool) {
        self.set_control_flag(CONTROL_LED_ON, led_on);
    }

    pub fn set_speed_zone(&mut self, speed_zone: Drive1541SpeedZone) {
        self.selected_speed_zone = speed_zone;
        self.gcr_circuit.set_speed_zone(speed_zone);
    }

    pub fn set_read_mode(&mut self, reading: bool) {
        if self.reading() == reading {
            return;
        }
        self.set_control_flag(CONTROL_READING, reading);
        self.gcr_circuit.set_read_mode(reading);
        self.byte_ready.set_asserted(false);
    }

    pub fn set_byte_ready_enabled(&mut self, enabled: bool) {
        self.set_control_flag(CONTROL_BYTE_READY_ENABLED, enabled);
        self.gcr_circuit.set_byte_ready_enabled(enabled);
        if !enabled {
            self.byte_ready.set_asserted(false);
        }
    }

    pub fn set_write_data_byte(&mut self, value: u8) {
        self.gcr_circuit.set_write_data_byte(value);
    }

    pub fn acknowledge_byte_ready(&mut self) {
        self.byte_ready.set_asserted(false);
    }

    /// Advance exact 1 MHz drive CPU cycles.
    ///
    /// # Errors
    ///
    /// Propagates checked-integer, media and GCR event failures.
    pub fn tick(&mut self, cpu_cycles: u64) -> Result<(), Drive1541MechanismError> {
        if cpu_cycles == 0 {
            return Ok(());
        }
        let head_blocked_cycles = self.disk_head_blocked_cycles(cpu_cycles);
        self.advance_disk_change(cpu_cycles);
        let rotating_cycles = cpu_cycles - head_blocked_cycles;
        if !self.motor_on() || rotating_cycles == 0 {
            return Ok(());
        }
        let reference_ticks = rotating_cycles
            .checked_mul(GCR_REFERENCE_TICKS_PER_CPU_CYCLE)
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
        if self.reading() {
            self.advance_read_rotation(reference_ticks)
        } else {
            self.advance_write_rotation(reference_ticks)
        }
    }

    /// Clone a materialized raw half-track for diagnostics or explicit export.
    ///
    /// # Errors
    ///
    /// Rejects invalid half-tracks, empty mechanisms and codec failures.
    pub fn read_raw_half_track(
        &mut self,
        half_track: u8,
    ) -> Result<Vec<u8>, Drive1541MechanismError> {
        Ok(self.media.raw_track_mut(half_track)?.clone())
    }

    /// Reset electronics without moving the head or rotating the disk.
    pub fn reset_electronics(&mut self) {
        self.gcr_circuit.reset();
        self.media_rotation_accumulator = 0;
        self.byte_ready.set_asserted(false);
    }

    fn move_head(&mut self, direction: i8) -> Result<(), Drive1541MechanismError> {
        let previous_track_bits = self.media.track_size_bits(self.current_half_track)?;
        let next_half_track = if direction > 0 {
            self.current_half_track
                .saturating_add(1)
                .min(DRIVE_1541_MAXIMUM_HALF_TRACK)
        } else {
            self.current_half_track
                .saturating_sub(1)
                .max(DRIVE_1541_MINIMUM_HALF_TRACK)
        };
        let next_track_bits = self.media.track_size_bits(next_half_track)?;
        let revolution = u128::from(REFERENCE_TICKS_PER_REVOLUTION);
        let previous_bits = u128::try_from(previous_track_bits)
            .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
        let next_bits = u128::try_from(next_track_bits)
            .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
        let angular = u128::try_from(self.angular_bit_offset)
            .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
        let phase = angular
            .checked_mul(revolution)
            .and_then(|value| value.checked_add(u128::from(self.media_rotation_accumulator)))
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
        let numerator = phase
            .checked_mul(next_bits)
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
        let denominator = previous_bits
            .checked_mul(revolution)
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
        let next_angular = (numerator / denominator).min(next_bits - 1);
        let next_accumulator = (numerator % denominator) / previous_bits;
        self.current_half_track = next_half_track;
        self.angular_bit_offset = usize::try_from(next_angular)
            .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
        self.media_rotation_accumulator = u64::try_from(next_accumulator)
            .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
        Ok(())
    }

    fn advance_read_rotation(
        &mut self,
        reference_ticks: u64,
    ) -> Result<(), Drive1541MechanismError> {
        let mut remaining = reference_ticks;
        while remaining > 0 {
            let track_bits = self.media.track_size_bits(self.current_half_track)?;
            let track_bits_u64 = u64::try_from(track_bits)
                .map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
            let phase_remaining = REFERENCE_TICKS_PER_REVOLUTION
                .checked_sub(self.media_rotation_accumulator)
                .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
            let ticks_until_next_cell = phase_remaining.div_ceil(track_bits_u64);
            let interval = remaining.min(ticks_until_next_cell);
            self.advance_gcr(interval)?;
            self.advance_media_phase(interval, track_bits)?;
            remaining -= interval;

            if self.media_rotation_accumulator < REFERENCE_TICKS_PER_REVOLUTION {
                if self.sync_found() {
                    self.byte_ready.set_asserted(false);
                }
                continue;
            }
            self.media_rotation_accumulator -= REFERENCE_TICKS_PER_REVOLUTION;
            if self
                .media
                .read_current_bit(self.current_half_track, self.angular_bit_offset)?
            {
                self.gcr_circuit.observe_recorded_flux_reversal();
            }
            self.advance_angular_position()?;
            if self.sync_found() {
                self.byte_ready.set_asserted(false);
            }
        }
        Ok(())
    }

    fn advance_write_rotation(
        &mut self,
        reference_ticks: u64,
    ) -> Result<(), Drive1541MechanismError> {
        let mut remaining = reference_ticks;
        while remaining > 0 {
            let interval = remaining.min(self.gcr_circuit.reference_ticks_until_next_shift());
            let track_bits = self.media.track_size_bits(self.current_half_track)?;
            self.advance_media_phase(interval, track_bits)?;
            self.advance_gcr(interval)?;
            remaining -= interval;
        }
        Ok(())
    }

    fn advance_media_phase(
        &mut self,
        reference_ticks: u64,
        track_bits: usize,
    ) -> Result<(), Drive1541MechanismError> {
        let track_bits =
            u64::try_from(track_bits).map_err(|_| Drive1541MechanismError::ArithmeticOverflow)?;
        let advance = reference_ticks
            .checked_mul(track_bits)
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
        self.media_rotation_accumulator = self
            .media_rotation_accumulator
            .checked_add(advance)
            .ok_or(Drive1541MechanismError::ArithmeticOverflow)?;
        if !self.reading() {
            self.media_rotation_accumulator %= REFERENCE_TICKS_PER_REVOLUTION;
        }
        Ok(())
    }

    fn advance_gcr(&mut self, reference_ticks: u64) -> Result<(), Drive1541MechanismError> {
        let mut signals = MechanismGcrSignals {
            media: &mut self.media,
            byte_ready: &mut self.byte_ready,
            angular_bit_offset: &mut self.angular_bit_offset,
            media_rotation_accumulator: &mut self.media_rotation_accumulator,
            current_half_track: self.current_half_track,
            selected_speed_zone: self.selected_speed_zone,
            error: None,
        };
        self.gcr_circuit.advance(reference_ticks, &mut signals)?;
        if let Some(error) = signals.error {
            return Err(error);
        }
        Ok(())
    }

    fn advance_angular_position(&mut self) -> Result<(), Drive1541MechanismError> {
        let track_bits = self.media.track_size_bits(self.current_half_track)?;
        self.angular_bit_offset = (self.angular_bit_offset + 1) % track_bits;
        Ok(())
    }

    fn finish_disk_removal(&mut self) {
        self.disk_insertion_cycles_remaining = 0;
        self.disk_replacement_gap_cycles_remaining = 0;
        self.disk_removal_cycles_remaining = DRIVE_1541_DISK_REMOVAL_CYCLES;
        self.media.clear_raw_tracks();
        self.reset_read_path_at_current_position();
    }

    fn disk_head_blocked_cycles(&self, cpu_cycles: u64) -> u64 {
        if self.media.disk.is_none() {
            return 0;
        }
        cpu_cycles.min(
            self.disk_insertion_cycles_remaining
                .max(self.disk_replacement_gap_cycles_remaining),
        )
    }

    fn advance_disk_change(&mut self, cpu_cycles: u64) {
        self.disk_insertion_cycles_remaining = self
            .disk_insertion_cycles_remaining
            .saturating_sub(cpu_cycles);
        self.disk_removal_cycles_remaining = self
            .disk_removal_cycles_remaining
            .saturating_sub(cpu_cycles);
        self.disk_replacement_gap_cycles_remaining = self
            .disk_replacement_gap_cycles_remaining
            .saturating_sub(cpu_cycles);
    }

    fn reset_read_path_at_current_position(&mut self) {
        self.angular_bit_offset = 0;
        self.media_rotation_accumulator = 0;
        self.gcr_circuit.reset();
        self.byte_ready.set_asserted(false);
    }

    const fn control_flag(&self, flag: u8) -> bool {
        self.control_flags & flag != 0
    }

    fn set_control_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.control_flags |= flag;
        } else {
            self.control_flags &= !flag;
        }
    }
}

fn require_half_track(half_track: u8) -> Result<(), Drive1541MechanismError> {
    if !(DRIVE_1541_MINIMUM_HALF_TRACK..=DRIVE_1541_MAXIMUM_HALF_TRACK).contains(&half_track) {
        return Err(Drive1541MechanismError::InvalidHalfTrack(half_track));
    }
    Ok(())
}

fn half_track_index(half_track: u8) -> Result<usize, Drive1541MechanismError> {
    require_half_track(half_track)?;
    Ok(usize::from(half_track - DRIVE_1541_MINIMUM_HALF_TRACK))
}

const fn to_g64_speed_zone(speed_zone: Drive1541SpeedZone) -> G64SpeedZone {
    match speed_zone {
        Drive1541SpeedZone::Zone0 => G64SpeedZone::Zone0,
        Drive1541SpeedZone::Zone1 => G64SpeedZone::Zone1,
        Drive1541SpeedZone::Zone2 => G64SpeedZone::Zone2,
        Drive1541SpeedZone::Zone3 => G64SpeedZone::Zone3,
    }
}
