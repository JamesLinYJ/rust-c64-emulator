// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - MOS 6526 serial shift register
//
//   File:       serial.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

const SERIAL_BYTE_BITS: u8 = 8;
const SERIAL_TRANSFER_HALF_BITS: u8 = SERIAL_BYTE_BITS * 2;
const SERIAL_MOST_SIGNIFICANT_BIT: u8 = 0x80;
const SERIAL_INTERRUPT_DELAY_CYCLES: u8 = 2;
const OUTPUT_REGISTER_LOAD_PIPELINE_INPUT: u8 = 1 << 1;
const OUTPUT_CLOCK_PIPELINE_INPUT: u8 = 1 << 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub(super) struct Mos6526SerialPort {
    buffered_output_byte: Option<u8>,
    input_bits_received: u8,
    input_shift_register: u8,
    interrupt_delay_cycles: Option<u8>,
    output_clock_high: bool,
    output_data_high: bool,
    output_data_register: u8,
    output_clock_pipeline: u8,
    output_half_bits_remaining: u8,
    output_load_pipeline: u8,
    output_shift_register: u8,
    seamless_output_byte: Option<u8>,
}

impl Default for Mos6526SerialPort {
    fn default() -> Self {
        Self::new()
    }
}

impl Mos6526SerialPort {
    pub(super) const fn new() -> Self {
        Self {
            buffered_output_byte: None,
            input_bits_received: 0,
            input_shift_register: 0,
            interrupt_delay_cycles: None,
            output_clock_high: true,
            output_data_high: true,
            output_data_register: 0,
            output_clock_pipeline: 0,
            output_half_bits_remaining: 0,
            output_load_pipeline: 0,
            output_shift_register: 0,
            seamless_output_byte: None,
        }
    }

    pub(super) const fn clock_output_high(self) -> bool {
        self.output_clock_high
    }

    pub(super) const fn data_output_high(self) -> bool {
        self.output_data_high
    }

    pub(super) const fn output_active(self) -> bool {
        self.output_half_bits_remaining > 0
    }

    pub(super) const fn cycle_work_pending(self) -> bool {
        self.output_load_pipeline != 0
            || self.output_clock_pipeline != 0
            || self.interrupt_delay_cycles.is_some()
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(super) fn write_output_byte(&mut self, value: u8) {
        self.output_data_register = value;
        self.output_load_pipeline |= OUTPUT_REGISTER_LOAD_PIPELINE_INPUT;
    }

    pub(super) fn schedule_output_clock_transition(&mut self) {
        if self.output_active() || self.buffered_output_byte.is_some() {
            self.output_clock_pipeline |= OUTPUT_CLOCK_PIPELINE_INPUT;
        }
    }

    pub(super) fn clock_output_half_bit(&mut self) -> bool {
        if !self.output_active() {
            return false;
        }

        self.output_clock_high = !self.output_clock_high;
        self.output_half_bits_remaining -= 1;
        if self.output_half_bits_remaining == 1 {
            self.interrupt_delay_cycles = Some(SERIAL_INTERRUPT_DELAY_CYCLES);
            if self.seamless_output_byte.is_none() && self.buffered_output_byte.is_some() {
                self.seamless_output_byte = self.buffered_output_byte.take();
            }
        }
        if self.output_clock_high {
            self.output_shift_register <<= 1;
            self.output_data_high = self.output_shift_register & SERIAL_MOST_SIGNIFICANT_BIT != 0;
        }
        if self.output_half_bits_remaining > 0 {
            return false;
        }

        let seamless = self.seamless_output_byte.take();
        let next = seamless.or(self.buffered_output_byte);
        if seamless.is_none() {
            self.buffered_output_byte = None;
        }
        if let Some(value) = next {
            self.load_output_shift_register(value);
        } else {
            self.output_data_high = true;
        }
        true
    }

    pub(super) fn tick_cycle(&mut self) -> bool {
        if !self.cycle_work_pending() {
            return false;
        }

        let output_load_due = self.output_load_pipeline & 1 != 0;
        self.output_load_pipeline >>= 1;
        if output_load_due {
            self.load_output_data_register();
        }

        let mut interrupt_raised = false;
        if let Some(remaining) = self.interrupt_delay_cycles {
            if remaining > 1 {
                self.interrupt_delay_cycles = Some(remaining - 1);
            } else {
                self.interrupt_delay_cycles = None;
                interrupt_raised = true;
            }
        }

        let output_clock_due = self.output_clock_pipeline & 1 != 0;
        self.output_clock_pipeline >>= 1;
        if output_clock_due {
            self.clock_output_half_bit();
        }
        interrupt_raised
    }

    pub(super) fn clock_input_bit(&mut self, input_high: bool) -> (bool, u8) {
        self.input_shift_register = (self.input_shift_register << 1) | u8::from(input_high);
        self.input_bits_received += 1;
        if self.input_bits_received < SERIAL_BYTE_BITS {
            return (false, self.input_shift_register);
        }
        self.input_bits_received = 0;
        (true, self.input_shift_register)
    }

    fn load_output_shift_register(&mut self, value: u8) {
        self.output_shift_register = value;
        self.output_half_bits_remaining = SERIAL_TRANSFER_HALF_BITS;
        self.output_clock_high = true;
        self.output_data_high = value & SERIAL_MOST_SIGNIFICANT_BIT != 0;
    }

    fn load_output_data_register(&mut self) {
        if !self.output_active() {
            self.load_output_shift_register(self.output_data_register);
        } else if self.output_half_bits_remaining == 1 && self.seamless_output_byte.is_none() {
            self.seamless_output_byte = Some(self.output_data_register);
        } else {
            self.buffered_output_byte = Some(self.output_data_register);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Mos6526SerialPort;

    #[test]
    fn idle_processor_cycle_is_an_exact_noop() {
        let mut serial = Mos6526SerialPort::new();
        let initial = serial;

        for _ in 0..1_000 {
            assert!(!serial.tick_cycle());
        }

        assert_eq!(serial, initial);
    }

    fn load_output_register(serial: &mut Mos6526SerialPort, value: u8) {
        serial.write_output_byte(value);
        assert!(!serial.output_active());
        assert!(!serial.tick_cycle());
        assert!(!serial.output_active());
        assert!(!serial.tick_cycle());
        assert!(serial.output_active());
    }

    #[test]
    fn output_is_msb_first_and_uses_two_half_bits_per_data_bit() {
        let mut serial = Mos6526SerialPort::new();
        load_output_register(&mut serial, 0xa5);
        let mut output = [0_u8; 8];
        for (bit, sampled) in output.iter_mut().enumerate() {
            *sampled = u8::from(serial.data_output_high());
            assert!(!serial.clock_output_half_bit());
            assert!(!serial.clock_output_high());
            assert_eq!(serial.clock_output_half_bit(), bit == 7);
            assert!(serial.clock_output_high());
        }
        assert_eq!(output, [1, 0, 1, 0, 0, 1, 0, 1]);
        assert!(!serial.output_active());
        assert!(serial.data_output_high());
    }

    #[test]
    fn output_transition_and_completion_use_internal_delays() {
        let mut serial = Mos6526SerialPort::new();
        load_output_register(&mut serial, 0x80);
        serial.schedule_output_clock_transition();
        assert!(!serial.tick_cycle());
        assert!(serial.clock_output_high());
        assert!(!serial.tick_cycle());
        assert!(!serial.clock_output_high());

        for _ in 1..15 {
            serial.clock_output_half_bit();
        }
        assert!(!serial.tick_cycle());
        assert!(serial.tick_cycle());
    }

    #[test]
    fn input_assembles_eight_bits_msb_first() {
        let mut serial = Mos6526SerialPort::new();
        let bits = [true, true, false, false, false, false, true, true];
        for (index, bit) in bits.into_iter().enumerate() {
            let (completed, value) = serial.clock_input_bit(bit);
            assert_eq!(completed, index == 7);
            if completed {
                assert_eq!(value, 0xc3);
            }
        }
    }
}
