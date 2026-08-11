// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - cycle-exact MOS 6526/6526A CIA
//
//   File:       cia/mod.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

mod serial;
mod timer;

use serial::Mos6526SerialPort;
use timer::{Mos6526Timer, TimerInputMode};

pub const REGISTER_COUNT: usize = 0x10;

pub mod register {
    pub const PORT_A: u8 = 0x00;
    pub const PORT_B: u8 = 0x01;
    pub const DATA_DIRECTION_A: u8 = 0x02;
    pub const DATA_DIRECTION_B: u8 = 0x03;
    pub const TIMER_A_LOW: u8 = 0x04;
    pub const TIMER_A_HIGH: u8 = 0x05;
    pub const TIMER_B_LOW: u8 = 0x06;
    pub const TIMER_B_HIGH: u8 = 0x07;
    pub const TIME_OF_DAY_TENTHS: u8 = 0x08;
    pub const TIME_OF_DAY_SECONDS: u8 = 0x09;
    pub const TIME_OF_DAY_MINUTES: u8 = 0x0a;
    pub const TIME_OF_DAY_HOURS: u8 = 0x0b;
    pub const SERIAL_DATA: u8 = 0x0c;
    pub const INTERRUPT_CONTROL: u8 = 0x0d;
    pub const TIMER_A_CONTROL: u8 = 0x0e;
    pub const TIMER_B_CONTROL: u8 = 0x0f;
}

pub mod interrupt {
    pub const TIMER_A: u8 = 1 << 0;
    pub const TIMER_B: u8 = 1 << 1;
    pub const ALARM: u8 = 1 << 2;
    pub const SERIAL: u8 = 1 << 3;
    pub const FLAG: u8 = 1 << 4;
    pub const SOURCE_MASK: u8 = 0x1f;
    pub const SET_OR_PENDING: u8 = 1 << 7;
}

pub mod control {
    pub const START: u8 = 1 << 0;
    pub const PORT_B_OUTPUT: u8 = 1 << 1;
    pub const TOGGLE_OUTPUT: u8 = 1 << 2;
    pub const ONE_SHOT: u8 = 1 << 3;
    pub const FORCE_LOAD: u8 = 1 << 4;
    pub const TIMER_A_INPUT_MODE: u8 = 1 << 5;
    pub const SERIAL_OUTPUT_MODE: u8 = 1 << 6;
    pub const TIME_OF_DAY_50_HZ: u8 = 1 << 7;
    pub const TIMER_B_INPUT_MODE_MASK: u8 = 0x60;
    pub const ALARM_WRITE: u8 = 1 << 7;
}

const CONTROL_START: u8 = control::START;
const CONTROL_TOGGLE_OUTPUT: u8 = control::TOGGLE_OUTPUT;
const CONTROL_ONE_SHOT: u8 = control::ONE_SHOT;
const CONTROL_FORCE_LOAD: u8 = control::FORCE_LOAD;

const PORT_B_TIMER_A_OUTPUT: u8 = 1 << 6;
const PORT_B_TIMER_B_OUTPUT: u8 = 1 << 7;
const TOD_AFTERNOON_BIT: u8 = 1 << 7;
const TOD_HOUR_MASK: u8 = 0x1f;
const TOD_INPUT_PHASE_COUNT: u8 = 6;

const PIPELINE_ACKNOWLEDGE_STAGE_1: u16 = 0x0001;
const PIPELINE_ACKNOWLEDGE_STAGE_0: u16 = 0x0002;
const PIPELINE_ACKNOWLEDGE_CANCELLATION_WINDOW: u16 = 0x0004;
const PIPELINE_ACKNOWLEDGE_EXPIRED: u16 = 0x0008;
const PIPELINE_SET_DATA_BIT_STAGE_1: u16 = 0x0010;
const PIPELINE_SET_DATA_BIT_STAGE_0: u16 = 0x0020;
const PIPELINE_SET_DATA_BIT_EXPIRED: u16 = 0x0040;
const PIPELINE_RAISE_STAGE_1: u16 = 0x0100;
const PIPELINE_RAISE_STAGE_0: u16 = 0x0200;
const PIPELINE_RAISE_EXPIRED: u16 = 0x0400;
const PIPELINE_READ_STAGE_1: u16 = 0x2000;
const PIPELINE_READ_EXPIRED: u16 = 0x4000;
const PIPELINE_EXPIRED_MASK: u16 = PIPELINE_ACKNOWLEDGE_EXPIRED
    | PIPELINE_SET_DATA_BIT_EXPIRED
    | PIPELINE_RAISE_EXPIRED
    | PIPELINE_READ_EXPIRED;
const STATE_INTERRUPT_LINE_ASSERTED: u8 = 1 << 0;
const STATE_TIMER_B_READ_COLLISION: u8 = 1 << 1;
const STATE_TIME_OF_DAY_STOPPED: u8 = 1 << 2;
const STATE_COUNT_PIN_HIGH: u8 = 1 << 3;
const STATE_FLAG_PIN_HIGH: u8 = 1 << 4;
const STATE_PORT_CONTROL_OUTPUT_HIGH: u8 = 1 << 5;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum Mos6526Model {
    #[default]
    Original,
    Revised,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Mos6526Timing {
    pub processor_clock_hz: u32,
    pub time_of_day_input_hz: u32,
}

impl Default for Mos6526Timing {
    fn default() -> Self {
        Self::PAL
    }
}

impl Mos6526Timing {
    pub const PAL: Self = Self {
        processor_clock_hz: 985_248,
        time_of_day_input_hz: 50,
    };

    pub const NTSC: Self = Self {
        processor_clock_hz: 1_022_727,
        time_of_day_input_hz: 60,
    };

    const fn is_valid(self) -> bool {
        self.processor_clock_hz > 0 && self.time_of_day_input_hz > 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mos6526TimingError;

impl fmt::Display for Mos6526TimingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MOS 6526 clock frequencies must be positive")
    }
}

impl std::error::Error for Mos6526TimingError {}

/// MOS 6526 core independent of board-specific keyboard, joystick, IEC and VIC wiring.
/// External port levels are supplied on reads; all other state is deterministic and
/// owned here, including the original/revised interrupt pipeline distinction.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Mos6526 {
    registers: [u8; REGISTER_COUNT],
    timer_a: Mos6526Timer,
    timer_b: Mos6526Timer,
    time_of_day: [u8; 4],
    alarm: [u8; 4],
    time_of_day_read_latch: Option<[u8; 4]>,
    interrupt_flags: u8,
    interrupt_mask: u8,
    acknowledged_interrupt_flags: u8,
    new_interrupt_flags: u8,
    interrupt_pipeline: u16,
    elapsed_cycle_count: u64,
    last_interrupt_control_read_cycle: Option<u64>,
    serial_port: Mos6526SerialPort,
    time_of_day_phase_accumulator: u64,
    time_of_day_divider_phase: u8,
    port_control_pulse_cycles_remaining: u8,
    state_flags: u8,
    model: Mos6526Model,
    timing: Mos6526Timing,
}

impl Default for Mos6526 {
    fn default() -> Self {
        Self::new_with_valid_timing(Mos6526Model::Original, Mos6526Timing::PAL)
    }
}

impl Mos6526 {
    /// Construct a CIA with explicit silicon model and exact integer clock ratio.
    ///
    /// # Errors
    ///
    /// Returns `Mos6526TimingError` when either frequency is zero.
    pub fn new(model: Mos6526Model, timing: Mos6526Timing) -> Result<Self, Mos6526TimingError> {
        if !timing.is_valid() {
            return Err(Mos6526TimingError);
        }
        Ok(Self::new_with_valid_timing(model, timing))
    }

    pub(crate) fn new_with_valid_timing(model: Mos6526Model, timing: Mos6526Timing) -> Self {
        debug_assert!(timing.is_valid());
        Self {
            registers: [0; REGISTER_COUNT],
            timer_a: Mos6526Timer::new(),
            timer_b: Mos6526Timer::new(),
            time_of_day: [0; 4],
            alarm: [0; 4],
            time_of_day_read_latch: None,
            interrupt_flags: 0,
            interrupt_mask: 0,
            acknowledged_interrupt_flags: 0,
            new_interrupt_flags: 0,
            interrupt_pipeline: 0,
            elapsed_cycle_count: 0,
            last_interrupt_control_read_cycle: None,
            serial_port: Mos6526SerialPort::new(),
            time_of_day_phase_accumulator: 0,
            time_of_day_divider_phase: 0,
            port_control_pulse_cycles_remaining: 0,
            state_flags: STATE_TIME_OF_DAY_STOPPED
                | STATE_COUNT_PIN_HIGH
                | STATE_FLAG_PIN_HIGH
                | STATE_PORT_CONTROL_OUTPUT_HIGH,
            model,
            timing,
        }
    }

    pub const fn model(&self) -> Mos6526Model {
        self.model
    }

    pub const fn interrupt_pending(&self) -> bool {
        self.state_flag(STATE_INTERRUPT_LINE_ASSERTED)
    }

    pub const fn port_a_output_latch(&self) -> u8 {
        self.registers[register::PORT_A as usize]
    }

    pub const fn port_a_data_direction(&self) -> u8 {
        self.registers[register::DATA_DIRECTION_A as usize]
    }

    pub const fn port_b_output_latch(&self) -> u8 {
        self.registers[register::PORT_B as usize]
    }

    pub const fn port_b_data_direction(&self) -> u8 {
        self.registers[register::DATA_DIRECTION_B as usize]
    }

    pub const fn port_a_output_pins(&self) -> u8 {
        Self::compose_port_output(self.port_a_output_latch(), self.port_a_data_direction())
    }

    pub fn port_b_output_pins(&self) -> u8 {
        let mut pins =
            Self::compose_port_output(self.port_b_output_latch(), self.port_b_data_direction());
        if self.registers[register::TIMER_A_CONTROL as usize] & control::PORT_B_OUTPUT != 0 {
            pins = set_pin_level(pins, PORT_B_TIMER_A_OUTPUT, self.timer_a.output_high());
        }
        if self.registers[register::TIMER_B_CONTROL as usize] & control::PORT_B_OUTPUT != 0 {
            pins = set_pin_level(pins, PORT_B_TIMER_B_OUTPUT, self.timer_b.output_high());
        }
        pins
    }

    pub const fn serial_clock_output_high(&self) -> bool {
        self.serial_port.clock_output_high()
    }

    pub const fn serial_data_output_high(&self) -> bool {
        self.serial_port.data_output_high()
    }

    pub const fn port_control_output_high(&self) -> bool {
        self.state_flag(STATE_PORT_CONTROL_OUTPUT_HIGH)
    }

    pub fn reset(&mut self) {
        // /RESET does not drive the external FLAG pin; preserve its physical level.
        let flag_pin_state = self.state_flags & STATE_FLAG_PIN_HIGH;
        self.registers.fill(0);
        self.timer_a.reset();
        self.timer_b.reset();
        self.time_of_day.fill(0);
        self.alarm.fill(0);
        self.time_of_day_read_latch = None;
        self.interrupt_flags = 0;
        self.interrupt_mask = 0;
        self.acknowledged_interrupt_flags = 0;
        self.new_interrupt_flags = 0;
        self.interrupt_pipeline = 0;
        self.elapsed_cycle_count = 0;
        self.last_interrupt_control_read_cycle = None;
        self.serial_port.reset();
        self.time_of_day_phase_accumulator = 0;
        self.time_of_day_divider_phase = 0;
        self.port_control_pulse_cycles_remaining = 0;
        self.state_flags = STATE_TIME_OF_DAY_STOPPED
            | STATE_COUNT_PIN_HIGH
            | STATE_PORT_CONTROL_OUTPUT_HIGH
            | flag_pin_state;
    }

    /// Read one of the sixteen mirrored registers with board-provided port inputs.
    pub fn read(&mut self, address: u16, external_port_a: u8, external_port_b: u8) -> u8 {
        let index = address.to_le_bytes()[0] & 0x0f;
        match index {
            register::PORT_A => self.port_a_output_pins() & external_port_a,
            register::PORT_B => {
                let value = self.port_b_output_pins() & external_port_b;
                self.trigger_port_control_output();
                value
            }
            register::TIMER_A_LOW => self.timer_a.counter().to_le_bytes()[0],
            register::TIMER_A_HIGH => self.timer_a.counter().to_le_bytes()[1],
            register::TIMER_B_LOW => self.timer_b.counter().to_le_bytes()[0],
            register::TIMER_B_HIGH => self.timer_b.counter().to_le_bytes()[1],
            register::TIME_OF_DAY_TENTHS
            | register::TIME_OF_DAY_SECONDS
            | register::TIME_OF_DAY_MINUTES
            | register::TIME_OF_DAY_HOURS => self.read_time_of_day(index),
            register::INTERRUPT_CONTROL => self.read_interrupt_control(),
            index => self.registers[index as usize],
        }
    }

    pub fn read_pulled_up(&mut self, address: u16) -> u8 {
        self.read(address, 0xff, 0xff)
    }

    /// Write one of the sixteen mirrored registers.
    pub fn write(&mut self, address: u16, value: u8) {
        let index = address.to_le_bytes()[0] & 0x0f;
        match index {
            register::PORT_A | register::DATA_DIRECTION_A | register::DATA_DIRECTION_B => {
                self.registers[index as usize] = value;
            }
            register::PORT_B => {
                self.registers[index as usize] = value;
                self.trigger_port_control_output();
            }
            register::TIMER_A_LOW => self.timer_a.write_latch_low(value),
            register::TIMER_A_HIGH => self.timer_a.write_latch_high(value),
            register::TIMER_B_LOW => self.timer_b.write_latch_low(value),
            register::TIMER_B_HIGH => self.timer_b.write_latch_high(value),
            register::TIME_OF_DAY_TENTHS
            | register::TIME_OF_DAY_SECONDS
            | register::TIME_OF_DAY_MINUTES
            | register::TIME_OF_DAY_HOURS => self.write_time_of_day(index, value),
            register::SERIAL_DATA => {
                self.registers[index as usize] = value;
                if self.registers[register::TIMER_A_CONTROL as usize] & control::SERIAL_OUTPUT_MODE
                    != 0
                {
                    self.serial_port.write_output_byte(value);
                }
            }
            register::INTERRUPT_CONTROL => self.write_interrupt_control(value),
            register::TIMER_A_CONTROL => self.write_timer_a_control(value),
            register::TIMER_B_CONTROL => self.write_timer_b_control(value),
            _ => unreachable!("CIA register is masked to four bits"),
        }
    }

    pub fn tick(&mut self, cycles: u64) -> bool {
        for _ in 0..cycles {
            self.run_processor_clock_cycle();
        }
        self.tick_time_of_day_from_processor_cycles(cycles);
        self.interrupt_pending()
    }

    pub fn clock_cycle(&mut self) -> bool {
        self.run_processor_clock_cycle();
        self.tick_time_of_day_from_processor_cycles(1);
        self.interrupt_pending()
    }

    pub fn pulse_count(&mut self, pulses: u64) -> bool {
        for _ in 0..pulses {
            if self.serial_port.tick_cycle() {
                self.raise_interrupt(interrupt::SERIAL);
            }
            let timer_a_underflow = self.timer_a.input_mode == TimerInputMode::CountPin
                && self.timer_a.tick_cycle(true);
            if timer_a_underflow {
                self.synchronize_stopped_timer_start_bit(register::TIMER_A_CONTROL, true);
                self.raise_interrupt(interrupt::TIMER_A);
                self.clock_serial_output();
            }
            let timer_b_step = self.timer_b.input_mode == TimerInputMode::CountPin
                || matches!(self.timer_b.input_mode, TimerInputMode::TimerAUnderflow)
                    && timer_a_underflow
                || self.timer_b.input_mode == TimerInputMode::TimerAUnderflowWhileCountHigh
                    && self.state_flag(STATE_COUNT_PIN_HIGH)
                    && timer_a_underflow;
            if timer_b_step && self.timer_b.tick_cycle(true) {
                self.synchronize_stopped_timer_start_bit(register::TIMER_B_CONTROL, false);
                self.raise_interrupt(interrupt::TIMER_B);
            }
            self.run_interrupt_pipeline_cycle();
        }
        self.interrupt_pending()
    }

    pub fn set_count_pin_high(&mut self, high: bool) {
        self.set_state_flag(STATE_COUNT_PIN_HIGH, high);
    }

    pub fn set_flag_pin_high(&mut self, high: bool) {
        if self.state_flag(STATE_FLAG_PIN_HIGH) && !high {
            self.raise_interrupt(interrupt::FLAG);
        }
        self.set_state_flag(STATE_FLAG_PIN_HIGH, high);
    }

    pub fn pulse_flag(&mut self) {
        self.set_flag_pin_high(false);
        self.set_flag_pin_high(true);
    }

    pub fn pulse_serial_clock(&mut self, input_bit: bool) {
        if self.registers[register::TIMER_A_CONTROL as usize] & control::SERIAL_OUTPUT_MODE != 0 {
            return;
        }
        let (completed, value) = self.serial_port.clock_input_bit(input_bit);
        if completed {
            self.registers[register::SERIAL_DATA as usize] = value;
            self.raise_interrupt(interrupt::SERIAL);
        }
    }

    pub fn tick_time_of_day_input(&mut self, pulses: u64) {
        if self.state_flag(STATE_TIME_OF_DAY_STOPPED) || pulses == 0 {
            return;
        }
        let terminal_phase =
            if self.registers[register::TIMER_A_CONTROL as usize] & control::TIME_OF_DAY_50_HZ != 0
            {
                4
            } else {
                5
            };
        let pulses_until_first_update = if self.time_of_day_divider_phase <= terminal_phase {
            u64::from(terminal_phase - self.time_of_day_divider_phase + 1)
        } else {
            u64::from(TOD_INPUT_PHASE_COUNT - self.time_of_day_divider_phase + terminal_phase + 1)
        };
        if pulses < pulses_until_first_update {
            let phase = (u64::from(self.time_of_day_divider_phase) + pulses)
                % u64::from(TOD_INPUT_PHASE_COUNT);
            self.time_of_day_divider_phase = phase.to_le_bytes()[0];
            return;
        }

        let pulses_after_first = pulses - pulses_until_first_update;
        let following_period = u64::from(terminal_phase + 1);
        let update_count = 1 + pulses_after_first / following_period;
        self.time_of_day_divider_phase = (pulses_after_first % following_period).to_le_bytes()[0];
        for _ in 0..update_count {
            self.increment_time_of_day();
        }
    }

    fn write_timer_a_control(&mut self, value: u8) {
        self.registers[register::TIMER_A_CONTROL as usize] = value & !control::FORCE_LOAD;
        let input_mode = if value & control::TIMER_A_INPUT_MODE == 0 {
            TimerInputMode::ProcessorClock
        } else {
            TimerInputMode::CountPin
        };
        self.timer_a.write_control(value, input_mode);
    }

    fn write_timer_b_control(&mut self, value: u8) {
        self.registers[register::TIMER_B_CONTROL as usize] = value & !control::FORCE_LOAD;
        self.timer_b
            .write_control(value, TimerInputMode::from_timer_b_control(value));
    }

    fn synchronize_stopped_timer_start_bit(&mut self, register_index: u8, timer_a: bool) {
        let running = if timer_a {
            self.timer_a.running()
        } else {
            self.timer_b.running()
        };
        if !running {
            self.registers[register_index as usize] &= !control::START;
        }
    }

    fn trigger_port_control_output(&mut self) {
        self.port_control_pulse_cycles_remaining = 1;
        self.set_state_flag(STATE_PORT_CONTROL_OUTPUT_HIGH, false);
    }

    fn tick_port_control_output(&mut self) {
        if self.port_control_pulse_cycles_remaining == 0 {
            return;
        }
        self.port_control_pulse_cycles_remaining -= 1;
        if self.port_control_pulse_cycles_remaining == 0 {
            self.set_state_flag(STATE_PORT_CONTROL_OUTPUT_HIGH, true);
        }
    }

    const fn compose_port_output(output: u8, direction: u8) -> u8 {
        (output & direction) | !direction
    }

    fn read_interrupt_control(&mut self) -> u8 {
        self.last_interrupt_control_read_cycle = Some(self.elapsed_cycle_count);
        if self.state_flag(STATE_TIMER_B_READ_COLLISION) {
            self.interrupt_flags &= !interrupt::TIMER_B;
            self.set_state_flag(STATE_TIMER_B_READ_COLLISION, false);
        }
        if self.model == Mos6526Model::Revised
            && self.interrupt_pipeline & PIPELINE_RAISE_STAGE_0 != 0
            && self.interrupt_flags & interrupt::SOURCE_MASK != 0
        {
            self.interrupt_flags |= interrupt::SET_OR_PENDING;
        }

        let result = self.interrupt_flags;
        self.interrupt_pipeline |= PIPELINE_ACKNOWLEDGE_STAGE_1;
        self.interrupt_pipeline &= !PIPELINE_RAISE_STAGE_0;
        if self.model == Mos6526Model::Revised {
            self.interrupt_pipeline &= !PIPELINE_SET_DATA_BIT_STAGE_0;
            let active =
                self.interrupt_flags & (interrupt::SOURCE_MASK | interrupt::SET_OR_PENDING);
            if active != 0 {
                self.acknowledged_interrupt_flags |= active | interrupt::SET_OR_PENDING;
            }
        } else {
            self.interrupt_flags &= interrupt::SET_OR_PENDING;
            self.new_interrupt_flags = 0;
        }
        self.set_state_flag(STATE_INTERRUPT_LINE_ASSERTED, false);
        result
    }

    fn write_interrupt_control(&mut self, value: u8) {
        let selected = value & interrupt::SOURCE_MASK;
        if value & interrupt::SET_OR_PENDING != 0 {
            self.interrupt_mask |= selected;
        } else {
            self.interrupt_mask &= !selected;
        }

        if self.interrupt_flags & self.interrupt_mask & interrupt::SOURCE_MASK != 0
            && !self.state_flag(STATE_INTERRUPT_LINE_ASSERTED)
        {
            if self.model == Mos6526Model::Revised
                && self.interrupt_pipeline & PIPELINE_READ_STAGE_1 == 0
            {
                self.interrupt_pipeline |= PIPELINE_RAISE_STAGE_0 | PIPELINE_SET_DATA_BIT_STAGE_0;
            } else {
                self.interrupt_pipeline |= PIPELINE_RAISE_STAGE_1 | PIPELINE_SET_DATA_BIT_STAGE_1;
            }
        } else if self.model == Mos6526Model::Original
            && self.interrupt_pipeline & PIPELINE_ACKNOWLEDGE_CANCELLATION_WINDOW != 0
        {
            self.interrupt_pipeline &= !(PIPELINE_RAISE_STAGE_0 | PIPELINE_SET_DATA_BIT_STAGE_0);
        }
    }

    fn raise_interrupt(&mut self, source: u8) {
        let source = source & interrupt::SOURCE_MASK;
        self.interrupt_flags |= source;
        self.new_interrupt_flags |= source;
        self.acknowledged_interrupt_flags &= !source;
    }

    fn run_interrupt_pipeline_cycle(&mut self) {
        if self.interrupt_pipeline == 0 && self.new_interrupt_flags == 0 {
            return;
        }
        let mut pipeline = self.interrupt_pipeline;
        if pipeline & PIPELINE_ACKNOWLEDGE_STAGE_0 != 0 {
            if self.model == Mos6526Model::Revised {
                self.interrupt_flags &= !self.acknowledged_interrupt_flags;
            } else {
                self.interrupt_flags &= !interrupt::SET_OR_PENDING;
            }
            self.acknowledged_interrupt_flags = 0;
        }
        if self.new_interrupt_flags & self.interrupt_mask != 0 {
            let follows_icr_read = self
                .last_interrupt_control_read_cycle
                .is_some_and(|cycle| cycle.saturating_add(1) == self.elapsed_cycle_count);
            if self.model == Mos6526Model::Revised && !follows_icr_read {
                pipeline |= PIPELINE_RAISE_STAGE_0 | PIPELINE_SET_DATA_BIT_STAGE_0;
            } else {
                pipeline |= PIPELINE_RAISE_STAGE_1 | PIPELINE_SET_DATA_BIT_STAGE_1;
            }
        }
        if pipeline & PIPELINE_SET_DATA_BIT_STAGE_0 != 0 {
            self.interrupt_flags |= interrupt::SET_OR_PENDING;
        }
        if pipeline & PIPELINE_RAISE_STAGE_0 != 0 {
            self.set_state_flag(STATE_INTERRUPT_LINE_ASSERTED, true);
        }
        self.new_interrupt_flags = 0;
        self.interrupt_pipeline = (pipeline << 1) & !PIPELINE_EXPIRED_MASK;
    }

    fn run_processor_clock_cycle(&mut self) {
        self.elapsed_cycle_count = self.elapsed_cycle_count.wrapping_add(1);
        self.tick_port_control_output();
        if self.serial_port.cycle_work_pending() && self.serial_port.tick_cycle() {
            self.raise_interrupt(interrupt::SERIAL);
        }

        let timer_a_underflow = self.timer_a.tick_cycle(false);
        if timer_a_underflow {
            self.synchronize_stopped_timer_start_bit(register::TIMER_A_CONTROL, true);
            self.raise_interrupt(interrupt::TIMER_A);
            self.clock_serial_output();
        }

        let cascade_timer_b = matches!(self.timer_b.input_mode, TimerInputMode::TimerAUnderflow)
            || self.timer_b.input_mode == TimerInputMode::TimerAUnderflowWhileCountHigh
                && self.state_flag(STATE_COUNT_PIN_HIGH);
        if self.timer_b.tick_cycle(false) {
            self.synchronize_stopped_timer_start_bit(register::TIMER_B_CONTROL, false);
            let read_collision = self.model == Mos6526Model::Original
                && self
                    .last_interrupt_control_read_cycle
                    .is_some_and(|cycle| cycle == self.elapsed_cycle_count.saturating_sub(1));
            self.set_state_flag(STATE_TIMER_B_READ_COLLISION, read_collision);
            self.raise_interrupt(interrupt::TIMER_B);
        }
        if cascade_timer_b && timer_a_underflow {
            self.timer_b.schedule_external_step();
        }
        self.run_interrupt_pipeline_cycle();
    }

    fn clock_serial_output(&mut self) {
        if self.registers[register::TIMER_A_CONTROL as usize] & control::SERIAL_OUTPUT_MODE != 0 {
            self.serial_port.schedule_output_clock_transition();
        }
    }

    fn tick_time_of_day_from_processor_cycles(&mut self, cycles: u64) {
        let accumulated = u128::from(self.time_of_day_phase_accumulator)
            + u128::from(cycles) * u128::from(self.timing.time_of_day_input_hz);
        let denominator = u128::from(self.timing.processor_clock_hz);
        let pulses = accumulated / denominator;
        self.time_of_day_phase_accumulator = u64::try_from(accumulated % denominator)
            .expect("TOD phase remainder is bounded by a 32-bit clock frequency");
        self.tick_time_of_day_input(u64::try_from(pulses).unwrap_or(u64::MAX));
    }

    fn read_time_of_day(&mut self, register_index: u8) -> u8 {
        let index = usize::from(register_index - register::TIME_OF_DAY_TENTHS);
        if register_index == register::TIME_OF_DAY_HOURS && self.time_of_day_read_latch.is_none() {
            self.time_of_day_read_latch = Some(self.time_of_day);
        }
        let value = self
            .time_of_day_read_latch
            .as_ref()
            .unwrap_or(&self.time_of_day)[index];
        if register_index == register::TIME_OF_DAY_TENTHS {
            self.time_of_day_read_latch = None;
        }
        value
    }

    fn write_time_of_day(&mut self, register_index: u8, value: u8) {
        let index = usize::from(register_index - register::TIME_OF_DAY_TENTHS);
        let normalized = normalize_time_of_day_value(register_index, value);
        let alarm_write =
            self.registers[register::TIMER_B_CONTROL as usize] & control::ALARM_WRITE != 0;
        if alarm_write {
            self.alarm[index] = normalized;
        } else {
            self.time_of_day[index] = normalized;
        }
        self.check_alarm();
        if !alarm_write {
            if register_index == register::TIME_OF_DAY_HOURS {
                self.set_state_flag(STATE_TIME_OF_DAY_STOPPED, true);
            }
            if register_index == register::TIME_OF_DAY_TENTHS {
                if self.state_flag(STATE_TIME_OF_DAY_STOPPED) {
                    self.time_of_day_divider_phase = 0;
                }
                self.set_state_flag(STATE_TIME_OF_DAY_STOPPED, false);
            }
        }
    }

    fn increment_time_of_day(&mut self) {
        if self.increment_time_of_day_tenths()
            && self.increment_time_of_day_minute_or_second(1)
            && self.increment_time_of_day_minute_or_second(2)
        {
            self.increment_time_of_day_hour();
        }
        self.check_alarm();
    }

    fn increment_time_of_day_tenths(&mut self) -> bool {
        let digit = self.time_of_day[0] & 0x0f;
        if digit == 9 {
            self.time_of_day[0] = 0;
            true
        } else {
            self.time_of_day[0] = digit.wrapping_add(1) & 0x0f;
            false
        }
    }

    fn increment_time_of_day_minute_or_second(&mut self, index: usize) -> bool {
        let encoded = self.time_of_day[index];
        let units = encoded & 0x0f;
        if units != 9 {
            self.time_of_day[index] = (encoded & 0x70) | (units.wrapping_add(1) & 0x0f);
            return false;
        }
        let tens = (encoded >> 4) & 0x07;
        if tens == 5 {
            self.time_of_day[index] = 0;
            true
        } else {
            self.time_of_day[index] = (tens.wrapping_add(1) & 0x07) << 4;
            false
        }
    }

    fn increment_time_of_day_hour(&mut self) {
        let encoded = self.time_of_day[3];
        let afternoon = encoded & TOD_AFTERNOON_BIT;
        let hour = encoded & TOD_HOUR_MASK;
        self.time_of_day[3] = match hour {
            0x09 => afternoon | 0x10,
            0x11 => (afternoon ^ TOD_AFTERNOON_BIT) | 0x12,
            0x12 => afternoon | 0x01,
            _ => afternoon | (hour & 0x10) | ((hour & 0x0f).wrapping_add(1) & 0x0f),
        };
    }

    fn check_alarm(&mut self) {
        if self.time_of_day == self.alarm {
            self.raise_interrupt(interrupt::ALARM);
        }
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

const fn set_pin_level(value: u8, mask: u8, high: bool) -> u8 {
    if high { value | mask } else { value & !mask }
}

const fn normalize_time_of_day_value(register_index: u8, value: u8) -> u8 {
    match register_index {
        register::TIME_OF_DAY_TENTHS => value & 0x0f,
        register::TIME_OF_DAY_SECONDS | register::TIME_OF_DAY_MINUTES => value & 0x7f,
        _ => value & (TOD_AFTERNOON_BIT | TOD_HOUR_MASK),
    }
}

#[cfg(test)]
mod tests {
    use super::{Mos6526, Mos6526Model, Mos6526Timing, control, interrupt, register};

    fn write_timer_a(cia: &mut Mos6526, value: u16) {
        let [low, high] = value.to_le_bytes();
        cia.write(u16::from(register::TIMER_A_LOW), low);
        cia.write(u16::from(register::TIMER_A_HIGH), high);
    }

    fn write_timer_b(cia: &mut Mos6526, value: u16) {
        let [low, high] = value.to_le_bytes();
        cia.write(u16::from(register::TIMER_B_LOW), low);
        cia.write(u16::from(register::TIMER_B_HIGH), high);
    }

    #[test]
    fn ports_combine_latches_direction_and_timer_overrides() {
        let mut cia = Mos6526::default();
        cia.write(u16::from(register::PORT_A), 0x05);
        cia.write(u16::from(register::DATA_DIRECTION_A), 0x0f);
        assert_eq!(cia.port_a_output_pins(), 0xf5);
        assert_eq!(cia.read_pulled_up(u16::from(register::PORT_A)), 0xf5);

        cia.write(u16::from(register::PORT_B), 0xff);
        cia.write(u16::from(register::DATA_DIRECTION_B), 0x00);
        cia.write(u16::from(register::TIMER_A_CONTROL), control::PORT_B_OUTPUT);
        cia.write(u16::from(register::TIMER_B_CONTROL), control::PORT_B_OUTPUT);
        assert_eq!(cia.port_b_output_pins(), 0x3f);
    }

    #[test]
    fn port_control_pulses_low_for_one_chip_cycle() {
        let mut cia = Mos6526::default();
        cia.read_pulled_up(u16::from(register::PORT_B));
        assert!(!cia.port_control_output_high());
        cia.clock_cycle();
        assert!(cia.port_control_output_high());
    }

    #[test]
    fn timers_and_icr_follow_original_and_revised_latency() {
        let mut original = Mos6526::default();
        let mut revised = Mos6526::new(Mos6526Model::Revised, Mos6526Timing::PAL).unwrap();
        for cia in [&mut original, &mut revised] {
            write_timer_a(cia, 1);
            cia.write(
                u16::from(register::INTERRUPT_CONTROL),
                interrupt::SET_OR_PENDING | interrupt::TIMER_A,
            );
            cia.write(
                u16::from(register::TIMER_A_CONTROL),
                control::START | control::FORCE_LOAD,
            );
        }
        assert!(!original.tick(4));
        assert!(revised.tick(4));
        assert!(original.clock_cycle());
        assert_eq!(
            original.read_pulled_up(u16::from(register::INTERRUPT_CONTROL)),
            interrupt::SET_OR_PENDING | interrupt::TIMER_A
        );
        assert!(!original.interrupt_pending());
    }

    #[test]
    fn timer_a_underflow_is_queued_into_timer_b() {
        let mut cia = Mos6526::default();
        write_timer_a(&mut cia, 1);
        write_timer_b(&mut cia, 2);
        cia.write(
            u16::from(register::TIMER_A_CONTROL),
            control::START | control::FORCE_LOAD,
        );
        cia.write(
            u16::from(register::TIMER_B_CONTROL),
            control::START | control::FORCE_LOAD | (2 << 5),
        );
        cia.tick(8);
        assert_eq!(cia.read_pulled_up(u16::from(register::TIMER_B_LOW)), 0);
        cia.clock_cycle();
        assert_eq!(
            cia.read_pulled_up(u16::from(register::INTERRUPT_CONTROL)),
            interrupt::TIMER_A | interrupt::TIMER_B
        );
    }

    #[test]
    fn tod_uses_bcd_carry_alarm_and_exact_integer_clock_phase() {
        let mut cia = Mos6526::new(
            Mos6526Model::Original,
            Mos6526Timing {
                processor_clock_hz: 10,
                time_of_day_input_hz: 2,
            },
        )
        .unwrap();
        cia.write(
            u16::from(register::TIMER_A_CONTROL),
            control::TIME_OF_DAY_50_HZ,
        );
        cia.write(u16::from(register::TIME_OF_DAY_HOURS), 0x11);
        cia.write(u16::from(register::TIME_OF_DAY_MINUTES), 0x59);
        cia.write(u16::from(register::TIME_OF_DAY_SECONDS), 0x59);
        cia.write(u16::from(register::TIME_OF_DAY_TENTHS), 0x09);
        cia.tick(25);
        assert_eq!(
            cia.read_pulled_up(u16::from(register::TIME_OF_DAY_HOURS)),
            0x92
        );
        assert_eq!(
            cia.read_pulled_up(u16::from(register::TIME_OF_DAY_MINUTES)),
            0
        );
        assert_eq!(
            cia.read_pulled_up(u16::from(register::TIME_OF_DAY_SECONDS)),
            0
        );
        assert_eq!(
            cia.read_pulled_up(u16::from(register::TIME_OF_DAY_TENTHS)),
            0
        );
    }

    #[test]
    fn flag_only_detects_falling_edges() {
        let mut cia = Mos6526::default();
        cia.set_flag_pin_high(false);
        assert_eq!(
            cia.read_pulled_up(u16::from(register::INTERRUPT_CONTROL)) & interrupt::FLAG,
            interrupt::FLAG
        );
        cia.set_flag_pin_high(false);
        assert_eq!(
            cia.read_pulled_up(u16::from(register::INTERRUPT_CONTROL)) & interrupt::FLAG,
            0
        );
        cia.set_flag_pin_high(true);
        cia.set_flag_pin_high(false);
        assert_eq!(
            cia.read_pulled_up(u16::from(register::INTERRUPT_CONTROL)) & interrupt::FLAG,
            interrupt::FLAG
        );
    }
}
