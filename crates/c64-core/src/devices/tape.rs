// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1530 Datasette
//
//   File:       devices/tape.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::media::tap::{TapImage, TapImageError, TapPulse, WritableTapImage};

const TAP_EXTENDED_PULSE_THRESHOLD_CYCLES: u64 = 0xff * 8 + 7;
const TAP_SHORT_PULSE_CYCLE_QUANTUM: u64 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct DatasetteHostSignals {
    pub motor_active: bool,
    pub write_high: bool,
}

impl Default for DatasetteHostSignals {
    fn default() -> Self {
        Self {
            motor_active: false,
            write_high: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum DatasetteTransport {
    #[default]
    Stopped,
    Play,
    Record,
}

impl DatasetteTransport {
    pub const fn code(self) -> u8 {
        match self {
            Self::Stopped => 0,
            Self::Play => 1,
            Self::Record => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DatasetteClockResult {
    pub read_pulses: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum DatasetteTape {
    ReadOnly(TapImage),
    Writable(WritableTapImage),
}

impl DatasetteTape {
    pub fn pulses(&self) -> &[TapPulse] {
        match self {
            Self::ReadOnly(image) => image.pulses(),
            Self::Writable(image) => image.pulses(),
        }
    }

    pub const fn source_clock_hz(&self) -> u32 {
        match self {
            Self::ReadOnly(image) => image.source_clock_hz(),
            Self::Writable(image) => image.source_clock_hz(),
        }
    }

    pub const fn writable(&self) -> bool {
        matches!(self, Self::Writable(_))
    }

    pub const fn writable_image(&self) -> Option<&WritableTapImage> {
        match self {
            Self::ReadOnly(_) => None,
            Self::Writable(image) => Some(image),
        }
    }

    /// Serialize the mounted image without changing transport state.
    ///
    /// # Errors
    ///
    /// Propagates writable TAP serialization errors.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TapImageError> {
        match self {
            Self::ReadOnly(image) => Ok(image.to_bytes()),
            Self::Writable(image) => image.to_bytes(),
        }
    }

    fn writable_image_mut(&mut self) -> Option<&mut WritableTapImage> {
        match self {
            Self::ReadOnly(_) => None,
            Self::Writable(image) => Some(image),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum DatasetteError {
    InvalidTargetClock(u32),
    TapeAlreadyInserted,
    TapeNotInserted,
    TransportMustBeStopped,
    WriteProtected,
    InvalidPulseIndex { index: usize, pulse_count: usize },
    PulseShorterThanTargetCycle { pulse_index: usize },
    ClockConversionOverflow,
    ElapsedDurationOverflow,
    Tap(TapImageError),
}

impl fmt::Display for DatasetteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTargetClock(clock) => {
                write!(
                    formatter,
                    "Datasette target clock {clock} must be greater than zero"
                )
            }
            Self::TapeAlreadyInserted => {
                formatter.write_str("a TAP image is already inserted in the Datasette")
            }
            Self::TapeNotInserted => formatter.write_str("the Datasette is empty"),
            Self::TransportMustBeStopped => {
                formatter.write_str("stop the Datasette before changing its media position")
            }
            Self::WriteProtected => {
                formatter.write_str("the inserted TAP image is write-protected")
            }
            Self::InvalidPulseIndex { index, pulse_count } => write!(
                formatter,
                "Datasette pulse index {index} is outside 0..={pulse_count}"
            ),
            Self::PulseShorterThanTargetCycle { pulse_index } => write!(
                formatter,
                "TAP pulse {pulse_index} is shorter than one Datasette target cycle"
            ),
            Self::ClockConversionOverflow => {
                formatter.write_str("Datasette clock conversion exceeds the 64-bit cycle range")
            }
            Self::ElapsedDurationOverflow => {
                formatter.write_str("Datasette elapsed duration exceeds the 64-bit cycle range")
            }
            Self::Tap(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DatasetteError {}

impl From<TapImageError> for DatasetteError {
    fn from(error: TapImageError) -> Self {
        Self::Tap(error)
    }
}

/// Cycle-clocked C1530 transport. Physical port signals are represented as
/// values and events so the hot path does not allocate observer callbacks.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Commodore1530Datasette {
    elapsed_target_cycles: u64,
    host_signals: DatasetteHostSignals,
    pulse_cycles_remaining: u64,
    pulse_index: usize,
    record_clock_remainder: u64,
    record_cycles_since_edge: u64,
    record_quantization_remainder: u64,
    scaling_remainder: u64,
    sense_switch_closed: bool,
    tape: Option<DatasetteTape>,
    target_clock_hz: u32,
    transport: DatasetteTransport,
}

impl Commodore1530Datasette {
    /// Construct an empty Datasette clocked by the host system clock.
    ///
    /// # Errors
    ///
    /// Rejects a zero target clock.
    pub fn new(target_clock_hz: u32) -> Result<Self, DatasetteError> {
        if target_clock_hz == 0 {
            return Err(DatasetteError::InvalidTargetClock(target_clock_hz));
        }
        Ok(Self::new_with_valid_target_clock(target_clock_hz))
    }

    pub(crate) fn new_with_valid_target_clock(target_clock_hz: u32) -> Self {
        debug_assert!(target_clock_hz != 0);
        Self {
            elapsed_target_cycles: 0,
            host_signals: DatasetteHostSignals::default(),
            pulse_cycles_remaining: 0,
            pulse_index: 0,
            record_clock_remainder: 0,
            record_cycles_since_edge: 0,
            record_quantization_remainder: 0,
            scaling_remainder: 0,
            sense_switch_closed: false,
            tape: None,
            target_clock_hz,
            transport: DatasetteTransport::Stopped,
        }
    }

    pub const fn elapsed_target_cycles(&self) -> u64 {
        self.elapsed_target_cycles
    }

    pub const fn host_signals(&self) -> DatasetteHostSignals {
        self.host_signals
    }

    pub const fn motor_active(&self) -> bool {
        self.host_signals.motor_active
    }

    pub const fn mounted_tape(&self) -> Option<&DatasetteTape> {
        self.tape.as_ref()
    }

    pub const fn pulse_index(&self) -> usize {
        self.pulse_index
    }

    pub const fn sense_switch_closed(&self) -> bool {
        self.sense_switch_closed
    }

    pub const fn target_clock_hz(&self) -> u32 {
        self.target_clock_hz
    }

    pub const fn transport(&self) -> DatasetteTransport {
        self.transport
    }

    /// Insert media while the transport is stopped.
    ///
    /// # Errors
    ///
    /// Rejects duplicate media and moving transports.
    pub fn insert_tape(&mut self, tape: DatasetteTape) -> Result<(), DatasetteError> {
        if self.transport != DatasetteTransport::Stopped {
            return Err(DatasetteError::TransportMustBeStopped);
        }
        if self.tape.is_some() {
            return Err(DatasetteError::TapeAlreadyInserted);
        }
        self.tape = Some(tape);
        self.reset_position();
        Ok(())
    }

    /// Eject and return the mounted media.
    ///
    /// # Errors
    ///
    /// Rejects an empty Datasette or a moving transport.
    pub fn eject_tape(&mut self) -> Result<DatasetteTape, DatasetteError> {
        if self.transport != DatasetteTransport::Stopped {
            return Err(DatasetteError::TransportMustBeStopped);
        }
        let tape = self.tape.take().ok_or(DatasetteError::TapeNotInserted)?;
        self.reset_position();
        Ok(tape)
    }

    pub fn press_play(&mut self) {
        self.transport = DatasetteTransport::Play;
        self.sense_switch_closed = true;
    }

    /// Engage RECORD on writable media.
    ///
    /// # Errors
    ///
    /// Rejects missing or read-only media and requires a stopped transport.
    pub fn press_record(&mut self) -> Result<(), DatasetteError> {
        if self.transport != DatasetteTransport::Stopped {
            return Err(DatasetteError::TransportMustBeStopped);
        }
        let tape = self.tape.as_ref().ok_or(DatasetteError::TapeNotInserted)?;
        if !tape.writable() {
            return Err(DatasetteError::WriteProtected);
        }
        if self.host_signals.motor_active {
            self.begin_recording_window()?;
        } else {
            self.reset_recording_timing();
        }
        self.transport = DatasetteTransport::Record;
        self.sense_switch_closed = true;
        Ok(())
    }

    pub fn press_stop(&mut self) {
        self.stop_transport();
    }

    /// Rewind mounted media to its first pulse.
    ///
    /// # Errors
    ///
    /// Requires mounted media and a stopped transport.
    pub fn rewind_to_start(&mut self) -> Result<(), DatasetteError> {
        if self.transport != DatasetteTransport::Stopped {
            return Err(DatasetteError::TransportMustBeStopped);
        }
        if self.tape.is_none() {
            return Err(DatasetteError::TapeNotInserted);
        }
        self.reset_position();
        Ok(())
    }

    /// Seek to a physical TAP pulse boundary.
    ///
    /// # Errors
    ///
    /// Requires mounted media, a stopped transport and an in-range boundary.
    pub fn seek_pulse(&mut self, pulse_index: usize) -> Result<(), DatasetteError> {
        if self.transport != DatasetteTransport::Stopped {
            return Err(DatasetteError::TransportMustBeStopped);
        }
        let pulse_count = self
            .tape
            .as_ref()
            .ok_or(DatasetteError::TapeNotInserted)?
            .pulses()
            .len();
        if pulse_index > pulse_count {
            return Err(DatasetteError::InvalidPulseIndex {
                index: pulse_index,
                pulse_count,
            });
        }
        self.pulse_index = pulse_index;
        self.pulse_cycles_remaining = 0;
        self.scaling_remainder = 0;
        self.elapsed_target_cycles = 0;
        self.reset_recording_timing();
        Ok(())
    }

    /// Update the C64 MOTOR and WRITE output lines.
    ///
    /// # Errors
    ///
    /// Propagates writable-media errors when a motor start erases the tape tail
    /// or a rising WRITE edge records a pulse.
    pub fn set_host_signals(
        &mut self,
        signals: DatasetteHostSignals,
    ) -> Result<(), DatasetteError> {
        let previous = self.host_signals;
        if previous == signals {
            return Ok(());
        }
        if self.transport == DatasetteTransport::Record {
            if !previous.motor_active && signals.motor_active {
                self.begin_recording_window()?;
            } else if previous.motor_active && !signals.motor_active {
                self.reset_recording_timing();
            }
            if signals.motor_active && !previous.write_high && signals.write_high {
                self.record_flux_transition()?;
            }
        }
        self.host_signals = signals;
        Ok(())
    }

    /// Advance one host cycle.
    ///
    /// # Errors
    ///
    /// Propagates exact clock-conversion and elapsed-duration overflow errors.
    pub fn clock_cycle(&mut self) -> Result<DatasetteClockResult, DatasetteError> {
        self.clock_cycles(1)
    }

    /// Advance an integer number of host cycles without changing single-cycle
    /// pulse boundaries.
    ///
    /// # Errors
    ///
    /// Propagates exact clock-conversion and elapsed-duration overflow errors.
    pub fn clock_cycles(
        &mut self,
        mut cycles: u64,
    ) -> Result<DatasetteClockResult, DatasetteError> {
        let mut result = DatasetteClockResult::default();
        if cycles == 0 || self.tape.is_none() || !self.host_signals.motor_active {
            return Ok(result);
        }
        if self.transport == DatasetteTransport::Record {
            self.record_cycles_since_edge = self
                .record_cycles_since_edge
                .checked_add(cycles)
                .ok_or(DatasetteError::ElapsedDurationOverflow)?;
            self.elapsed_target_cycles = self
                .elapsed_target_cycles
                .checked_add(cycles)
                .ok_or(DatasetteError::ElapsedDurationOverflow)?;
            return Ok(result);
        }
        if self.transport != DatasetteTransport::Play {
            return Ok(result);
        }

        while cycles != 0 {
            let pulse_count = self.tape.as_ref().map_or(0, |tape| tape.pulses().len());
            if self.pulse_index >= pulse_count {
                self.stop_transport();
                break;
            }
            if self.pulse_cycles_remaining == 0 {
                self.load_current_pulse_duration()?;
            }
            let consumed = cycles.min(self.pulse_cycles_remaining);
            cycles -= consumed;
            self.pulse_cycles_remaining -= consumed;
            self.elapsed_target_cycles = self
                .elapsed_target_cycles
                .checked_add(consumed)
                .ok_or(DatasetteError::ElapsedDurationOverflow)?;
            if self.pulse_cycles_remaining != 0 {
                continue;
            }
            self.pulse_index += 1;
            result.read_pulses = result
                .read_pulses
                .checked_add(1)
                .ok_or(DatasetteError::ElapsedDurationOverflow)?;
            if self.pulse_index >= pulse_count {
                self.stop_transport();
            }
        }
        Ok(result)
    }

    /// Change the target clock while retaining physical media and position.
    /// Any partially converted pulse restarts at its current pulse boundary.
    ///
    /// # Errors
    ///
    /// Rejects a zero target clock.
    pub fn reconfigure_target_clock(&mut self, target_clock_hz: u32) -> Result<(), DatasetteError> {
        if target_clock_hz == 0 {
            return Err(DatasetteError::InvalidTargetClock(target_clock_hz));
        }
        self.target_clock_hz = target_clock_hz;
        self.pulse_cycles_remaining = 0;
        self.scaling_remainder = 0;
        self.reset_recording_timing();
        Ok(())
    }

    fn begin_recording_window(&mut self) -> Result<(), DatasetteError> {
        let image = self
            .tape
            .as_mut()
            .and_then(DatasetteTape::writable_image_mut)
            .ok_or(DatasetteError::WriteProtected)?;
        image.truncate_at_pulse(self.pulse_index)?;
        self.reset_recording_timing();
        Ok(())
    }

    fn load_current_pulse_duration(&mut self) -> Result<(), DatasetteError> {
        let tape = self.tape.as_ref().ok_or(DatasetteError::TapeNotInserted)?;
        let pulse =
            tape.pulses()
                .get(self.pulse_index)
                .ok_or(DatasetteError::InvalidPulseIndex {
                    index: self.pulse_index,
                    pulse_count: tape.pulses().len(),
                })?;
        let numerator = u64::from(pulse.source_cycles)
            .checked_mul(u64::from(self.target_clock_hz))
            .and_then(|value| value.checked_add(self.scaling_remainder))
            .ok_or(DatasetteError::ClockConversionOverflow)?;
        let source_clock_hz = u64::from(tape.source_clock_hz());
        self.pulse_cycles_remaining = numerator / source_clock_hz;
        self.scaling_remainder = numerator % source_clock_hz;
        if self.pulse_cycles_remaining == 0 {
            return Err(DatasetteError::PulseShorterThanTargetCycle {
                pulse_index: self.pulse_index,
            });
        }
        Ok(())
    }

    fn record_flux_transition(&mut self) -> Result<(), DatasetteError> {
        let source_clock_hz = self
            .tape
            .as_ref()
            .ok_or(DatasetteError::TapeNotInserted)?
            .source_clock_hz();
        let numerator = u128::from(self.record_cycles_since_edge)
            .checked_mul(u128::from(source_clock_hz))
            .and_then(|value| value.checked_add(u128::from(self.record_clock_remainder)))
            .ok_or(DatasetteError::ClockConversionOverflow)?;
        let converted_cycles = numerator / u128::from(self.target_clock_hz);
        let next_clock_remainder = u64::try_from(numerator % u128::from(self.target_clock_hz))
            .map_err(|_| DatasetteError::ClockConversionOverflow)?;
        let accumulated_cycles = converted_cycles
            .checked_add(u128::from(self.record_quantization_remainder))
            .ok_or(DatasetteError::ClockConversionOverflow)?;

        let (encoded_cycles, next_quantization_remainder) =
            if accumulated_cycles < u128::from(TAP_EXTENDED_PULSE_THRESHOLD_CYCLES) {
                let units = accumulated_cycles / u128::from(TAP_SHORT_PULSE_CYCLE_QUANTUM);
                if units == 0 {
                    self.record_cycles_since_edge = 0;
                    self.record_clock_remainder = next_clock_remainder;
                    self.record_quantization_remainder = u64::try_from(accumulated_cycles)
                        .map_err(|_| DatasetteError::ClockConversionOverflow)?;
                    return Ok(());
                }
                (
                    units * u128::from(TAP_SHORT_PULSE_CYCLE_QUANTUM),
                    accumulated_cycles % u128::from(TAP_SHORT_PULSE_CYCLE_QUANTUM),
                )
            } else {
                (accumulated_cycles, 0)
            };
        let encoded_cycles =
            u32::try_from(encoded_cycles).map_err(|_| DatasetteError::ClockConversionOverflow)?;
        let image = self
            .tape
            .as_mut()
            .and_then(DatasetteTape::writable_image_mut)
            .ok_or(DatasetteError::WriteProtected)?;
        image.append_pulse(encoded_cycles)?;

        self.record_cycles_since_edge = 0;
        self.record_clock_remainder = next_clock_remainder;
        self.record_quantization_remainder = u64::try_from(next_quantization_remainder)
            .map_err(|_| DatasetteError::ClockConversionOverflow)?;
        self.pulse_index = image.pulses().len();
        Ok(())
    }

    fn reset_position(&mut self) {
        self.pulse_index = 0;
        self.pulse_cycles_remaining = 0;
        self.scaling_remainder = 0;
        self.elapsed_target_cycles = 0;
        self.reset_recording_timing();
    }

    fn reset_recording_timing(&mut self) {
        self.record_cycles_since_edge = 0;
        self.record_clock_remainder = 0;
        self.record_quantization_remainder = 0;
    }

    fn stop_transport(&mut self) {
        self.transport = DatasetteTransport::Stopped;
        self.sense_switch_closed = false;
        self.reset_recording_timing();
    }
}
