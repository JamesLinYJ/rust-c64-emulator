// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 GCR read/write separator
//
//   File:       devices/drive1541/gcr.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const GCR_REFERENCE_CLOCK_HZ: u32 = 16_000_000;
pub const GCR_REFERENCE_TICKS_PER_CPU_CYCLE: u64 = 16;
const GCR_COUNTER_LIMIT: u64 = 16;
const FLUX_FILTER_STABLE_TICKS: u64 = 40;
const G64_PREFILTERED_FLUX_TICKS: u64 = 39;
const RANDOM_RESET_SEED: u32 = 0x1234_abcd;
const SHIFT_REGISTER_MASK: u16 = 0x03ff;
const SYNC_PATTERN: u16 = 0x03ff;
const WEAK_FLUX_INITIAL_DELAY_MINIMUM: u64 = 289;
const WEAK_FLUX_INITIAL_DELAY_RANGE: u32 = 31;
const WEAK_FLUX_REPEAT_DELAY_MINIMUM: u64 = 33;
const WEAK_FLUX_REPEAT_DELAY_RANGE: u32 = 367;
const SO_MINIMUM_DELAY_TICKS: u64 = 10;
const UF4_SHIFT_PHASE_MASK: u8 = 0x03;
const UF4_SHIFT_PHASE: u8 = 0x02;
const UF4_COUNTER_MASK: u8 = 0x0f;
const WRITE_DATA_MOST_SIGNIFICANT_BIT: u8 = 0x80;
const BITS_PER_BYTE: u8 = 8;
const STATE_READING: u8 = 1 << 0;
const STATE_BYTE_READY_ENABLED: u8 = 1 << 1;
const STATE_FLUX_HIGH: u8 = 1 << 2;
const STATE_ACCEPTED_FLUX_HIGH: u8 = 1 << 3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum Drive1541SpeedZone {
    #[default]
    Zone0 = 0,
    Zone1 = 1,
    Zone2 = 2,
    Zone3 = 3,
}

impl Drive1541SpeedZone {
    pub const ALL: [Self; 4] = [Self::Zone0, Self::Zone1, Self::Zone2, Self::Zone3];

    pub const fn get(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for Drive1541SpeedZone {
    type Error = Drive1541GcrError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Zone0),
            1 => Ok(Self::Zone1),
            2 => Ok(Self::Zone2),
            3 => Ok(Self::Zone3),
            _ => Err(Drive1541GcrError::InvalidSpeedZone(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Drive1541GcrError {
    DividerPassedCarry,
    NonPositiveEventInterval,
    SoDelayPassedEdge,
    WeakFluxPassedEdge,
    InvalidSpeedZone(u8),
}

impl fmt::Display for Drive1541GcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DividerPassedCarry => write!(formatter, "1541 UE7 divider passed its carry edge"),
            Self::NonPositiveEventInterval => {
                write!(
                    formatter,
                    "1541 GCR scheduler produced a zero event interval"
                )
            }
            Self::SoDelayPassedEdge => write!(formatter, "1541 GCR SO delay passed its edge"),
            Self::WeakFluxPassedEdge => {
                write!(formatter, "1541 GCR weak-flux countdown passed its edge")
            }
            Self::InvalidSpeedZone(value) => {
                write!(formatter, "1541 speed zone {value} is outside 0..=3")
            }
        }
    }
}

impl std::error::Error for Drive1541GcrError {}

pub trait Drive1541GcrSignals {
    fn signal_byte_ready(&mut self, data_byte: u8);
    fn write_flux_bit(&mut self, high: bool);
}

/// Cycle-exact UE7/UF4 divider, flux filter, ten-bit sync detector and UE3
/// byte counter. The state contains no host pointers and is deterministic.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Drive1541GcrCircuit {
    speed_zone: Drive1541SpeedZone,
    state_flags: u8,
    data_byte: u8,
    write_data_byte: u8,
    ue7_counter: u64,
    uf4_counter: u8,
    ten_bit_shift_register: u16,
    byte_bit_counter: u8,
    write_shift_register: u8,
    flux_filter_counter: u64,
    weak_flux_countdown: u64,
    random_state: u32,
    so_delay_ticks: u64,
    reference_clock_phase: u8,
}

impl Default for Drive1541GcrCircuit {
    fn default() -> Self {
        Self::new()
    }
}

impl Drive1541GcrCircuit {
    pub const fn new() -> Self {
        Self {
            speed_zone: Drive1541SpeedZone::Zone0,
            state_flags: STATE_READING | STATE_BYTE_READY_ENABLED,
            data_byte: 0,
            write_data_byte: 0,
            ue7_counter: 0,
            uf4_counter: 0,
            ten_bit_shift_register: 0,
            byte_bit_counter: 0,
            write_shift_register: 0,
            flux_filter_counter: 0,
            weak_flux_countdown: WEAK_FLUX_INITIAL_DELAY_MINIMUM,
            random_state: RANDOM_RESET_SEED,
            so_delay_ticks: 0,
            reference_clock_phase: 0,
        }
    }

    pub const fn speed_zone(&self) -> Drive1541SpeedZone {
        self.speed_zone
    }

    pub const fn reading(&self) -> bool {
        self.state_flag(STATE_READING)
    }

    pub const fn byte_ready_enabled(&self) -> bool {
        self.state_flag(STATE_BYTE_READY_ENABLED)
    }

    pub const fn data_byte(&self) -> u8 {
        self.data_byte
    }

    pub const fn write_data_byte(&self) -> u8 {
        self.write_data_byte
    }

    pub const fn sync_found(&self) -> bool {
        self.reading() && self.ten_bit_shift_register == SYNC_PATTERN
    }

    pub const fn weak_flux_ticks_remaining(&self) -> u64 {
        self.weak_flux_countdown
    }

    pub fn reference_ticks_until_next_shift(&self) -> u64 {
        let mut ticks = GCR_COUNTER_LIMIT - self.ue7_counter;
        let mut counter = self.uf4_counter;
        loop {
            counter = counter.wrapping_add(1) & UF4_COUNTER_MASK;
            if counter & UF4_SHIFT_PHASE_MASK == UF4_SHIFT_PHASE {
                return ticks;
            }
            ticks += GCR_COUNTER_LIMIT - u64::from(self.speed_zone.get());
        }
    }

    pub fn reset(&mut self) {
        self.data_byte = 0;
        self.ue7_counter = 0;
        self.uf4_counter = 0;
        self.ten_bit_shift_register = 0;
        self.byte_bit_counter = 0;
        self.write_shift_register = 0;
        self.flux_filter_counter = 0;
        self.set_state_flag(STATE_FLUX_HIGH, false);
        self.set_state_flag(STATE_ACCEPTED_FLUX_HIGH, false);
        self.weak_flux_countdown = WEAK_FLUX_INITIAL_DELAY_MINIMUM;
        self.random_state = RANDOM_RESET_SEED;
        self.so_delay_ticks = 0;
        self.reference_clock_phase = 0;
    }

    pub fn set_speed_zone(&mut self, speed_zone: Drive1541SpeedZone) {
        self.speed_zone = speed_zone;
    }

    pub fn set_read_mode(&mut self, reading: bool) {
        self.set_state_flag(STATE_READING, reading);
    }

    pub fn set_byte_ready_enabled(&mut self, enabled: bool) {
        self.set_state_flag(STATE_BYTE_READY_ENABLED, enabled);
    }

    pub fn set_write_data_byte(&mut self, value: u8) {
        self.write_data_byte = value;
    }

    pub fn observe_recorded_flux_reversal(&mut self) {
        if !self.reading() {
            return;
        }
        let next = !self.state_flag(STATE_FLUX_HIGH);
        self.set_state_flag(STATE_FLUX_HIGH, next);
        self.flux_filter_counter = G64_PREFILTERED_FLUX_TICKS;
    }

    /// Advance a number of 16 MHz reference ticks and deliver pin events in
    /// architectural order.
    ///
    /// # Errors
    ///
    /// Returns an error only if an internal event boundary is skipped.
    pub fn advance<S: Drive1541GcrSignals>(
        &mut self,
        reference_ticks: u64,
        signals: &mut S,
    ) -> Result<(), Drive1541GcrError> {
        let mut remaining = reference_ticks;
        while remaining > 0 {
            let interval = self.next_event_interval(remaining)?;
            self.advance_so_delay(interval, signals)?;
            if self.reading() {
                self.advance_read_separator(interval)?;
            }
            self.advance_divider(interval, signals)?;
            let phase = (u64::from(self.reference_clock_phase) + interval)
                & (GCR_REFERENCE_TICKS_PER_CPU_CYCLE - 1);
            self.reference_clock_phase = phase.to_le_bytes()[0];
            remaining -= interval;
        }
        Ok(())
    }

    fn next_event_interval(&self, remaining: u64) -> Result<u64, Drive1541GcrError> {
        let mut interval = remaining.min(GCR_COUNTER_LIMIT - self.ue7_counter);
        if self.so_delay_ticks > 0 {
            interval = interval.min(self.so_delay_ticks);
        }
        if self.reading() {
            if self.flux_filter_counter < FLUX_FILTER_STABLE_TICKS {
                interval = interval.min(FLUX_FILTER_STABLE_TICKS - self.flux_filter_counter);
            }
            interval = interval.min(self.weak_flux_countdown);
        }
        if interval == 0 {
            return Err(Drive1541GcrError::NonPositiveEventInterval);
        }
        Ok(interval)
    }

    fn advance_so_delay<S: Drive1541GcrSignals>(
        &mut self,
        reference_ticks: u64,
        signals: &mut S,
    ) -> Result<(), Drive1541GcrError> {
        if self.so_delay_ticks == 0 {
            return Ok(());
        }
        self.so_delay_ticks = self
            .so_delay_ticks
            .checked_sub(reference_ticks)
            .ok_or(Drive1541GcrError::SoDelayPassedEdge)?;
        if self.so_delay_ticks == 0 {
            signals.signal_byte_ready(self.data_byte);
        }
        Ok(())
    }

    fn advance_read_separator(&mut self, reference_ticks: u64) -> Result<(), Drive1541GcrError> {
        self.flux_filter_counter += reference_ticks;
        if self.flux_filter_counter >= FLUX_FILTER_STABLE_TICKS
            && self.state_flag(STATE_ACCEPTED_FLUX_HIGH) != self.state_flag(STATE_FLUX_HIGH)
        {
            self.set_state_flag(STATE_ACCEPTED_FLUX_HIGH, self.state_flag(STATE_FLUX_HIGH));
            self.reset_pulse_divider();
            self.weak_flux_countdown = self.next_weak_flux_delay(
                WEAK_FLUX_INITIAL_DELAY_MINIMUM,
                WEAK_FLUX_INITIAL_DELAY_RANGE,
            );
            return Ok(());
        }

        self.weak_flux_countdown = self
            .weak_flux_countdown
            .checked_sub(reference_ticks)
            .ok_or(Drive1541GcrError::WeakFluxPassedEdge)?;
        if self.weak_flux_countdown == 0 {
            self.reset_pulse_divider();
            self.weak_flux_countdown = self
                .next_weak_flux_delay(WEAK_FLUX_REPEAT_DELAY_MINIMUM, WEAK_FLUX_REPEAT_DELAY_RANGE);
        }
        Ok(())
    }

    fn reset_pulse_divider(&mut self) {
        self.ue7_counter = u64::from(self.speed_zone.get());
        self.uf4_counter = 0;
    }

    fn advance_divider<S: Drive1541GcrSignals>(
        &mut self,
        reference_ticks: u64,
        signals: &mut S,
    ) -> Result<(), Drive1541GcrError> {
        self.ue7_counter += reference_ticks;
        if self.ue7_counter < GCR_COUNTER_LIMIT {
            return Ok(());
        }
        if self.ue7_counter > GCR_COUNTER_LIMIT {
            return Err(Drive1541GcrError::DividerPassedCarry);
        }
        self.ue7_counter = u64::from(self.speed_zone.get());
        self.uf4_counter = self.uf4_counter.wrapping_add(1) & UF4_COUNTER_MASK;
        if self.uf4_counter & UF4_SHIFT_PHASE_MASK != UF4_SHIFT_PHASE {
            return Ok(());
        }
        self.shift_gcr_data(reference_ticks, signals);
        Ok(())
    }

    fn shift_gcr_data<S: Drive1541GcrSignals>(&mut self, reference_ticks: u64, signals: &mut S) {
        let decoded_bit = u16::from((self.uf4_counter.wrapping_add(0x1c) >> 4) & 1);
        self.ten_bit_shift_register =
            ((self.ten_bit_shift_register << 1) | decoded_bit) & SHIFT_REGISTER_MASK;

        let write_bit = self.write_shift_register & WRITE_DATA_MOST_SIGNIFICANT_BIT != 0;
        self.write_shift_register <<= 1;
        if !self.reading() {
            signals.write_flux_bit(write_bit);
            self.byte_bit_counter += 1;
            if self.byte_bit_counter == BITS_PER_BYTE {
                self.byte_bit_counter = 0;
                self.write_shift_register = self.write_data_byte;
                self.schedule_byte_ready(reference_ticks);
            }
            return;
        }

        if self.ten_bit_shift_register == SYNC_PATTERN {
            self.byte_bit_counter = 0;
            return;
        }
        self.byte_bit_counter += 1;
        if self.byte_bit_counter != BITS_PER_BYTE {
            return;
        }
        self.byte_bit_counter = 0;
        self.data_byte = self.ten_bit_shift_register.to_le_bytes()[0];
        self.write_shift_register = self.data_byte;
        self.schedule_byte_ready(reference_ticks);
    }

    fn schedule_byte_ready(&mut self, reference_ticks: u64) {
        if !self.byte_ready_enabled() {
            return;
        }
        let shift_tick_phase = (u64::from(self.reference_clock_phase) + reference_ticks - 1)
            & (GCR_REFERENCE_TICKS_PER_CPU_CYCLE - 1);
        let mut delay = GCR_REFERENCE_TICKS_PER_CPU_CYCLE - shift_tick_phase;
        if delay < SO_MINIMUM_DELAY_TICKS {
            delay += GCR_REFERENCE_TICKS_PER_CPU_CYCLE;
        }
        self.so_delay_ticks = delay;
    }

    fn next_weak_flux_delay(&mut self, minimum: u64, range: u32) -> u64 {
        minimum + u64::from((self.next_random_u32() >> 16) % range)
    }

    fn next_random_u32(&mut self) -> u32 {
        let mut value = self.random_state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.random_state = value;
        value
    }

    const fn state_flag(&self, flag: u8) -> bool {
        self.state_flags & flag != 0
    }

    fn set_state_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.state_flags |= flag;
        } else {
            self.state_flags &= !flag;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Drive1541GcrCircuit, Drive1541GcrSignals, Drive1541SpeedZone,
        GCR_REFERENCE_TICKS_PER_CPU_CYCLE,
    };

    #[derive(Default)]
    struct Signals {
        byte_ready: Vec<u8>,
        write_bits: Vec<bool>,
    }

    impl Drive1541GcrSignals for Signals {
        fn signal_byte_ready(&mut self, data_byte: u8) {
            self.byte_ready.push(data_byte);
        }

        fn write_flux_bit(&mut self, high: bool) {
            self.write_bits.push(high);
        }
    }

    #[test]
    fn write_mode_shifts_a_byte_and_delays_byte_ready_to_the_cpu_phase() {
        let mut circuit = Drive1541GcrCircuit::new();
        let mut signals = Signals::default();
        circuit.set_speed_zone(Drive1541SpeedZone::Zone3);
        circuit.set_read_mode(false);
        circuit.set_write_data_byte(0xa5);
        for _ in 0..40 {
            circuit
                .advance(GCR_REFERENCE_TICKS_PER_CPU_CYCLE, &mut signals)
                .unwrap();
        }
        assert!(signals.write_bits.len() >= 8);
        assert!(!signals.byte_ready.is_empty());
    }

    #[test]
    fn reset_restores_the_deterministic_weak_flux_sequence() {
        let mut circuit = Drive1541GcrCircuit::new();
        let mut first = Signals::default();
        circuit.advance(20_000, &mut first).unwrap();
        let first_data = circuit.data_byte();
        let first_delay = circuit.weak_flux_ticks_remaining();

        circuit.reset();
        let mut second = Signals::default();
        circuit.advance(20_000, &mut second).unwrap();
        assert_eq!(circuit.data_byte(), first_data);
        assert_eq!(circuit.weak_flux_ticks_remaining(), first_delay);
        assert_eq!(first.byte_ready, second.byte_ready);
    }
}
