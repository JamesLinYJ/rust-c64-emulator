// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - cycle-exact MOS 6522 VIA
//
//   File:       devices/via.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

pub const MOS6522_REGISTER_COUNT: usize = 0x10;
const FINISHED_SHIFT_PHASE: u8 = 16;
const PORT_B_7_BIT: u8 = 1 << 7;

pub mod register {
    pub const PORT_B: u8 = 0x00;
    pub const PORT_A: u8 = 0x01;
    pub const DATA_DIRECTION_B: u8 = 0x02;
    pub const DATA_DIRECTION_A: u8 = 0x03;
    pub const TIMER_1_COUNTER_LOW: u8 = 0x04;
    pub const TIMER_1_COUNTER_HIGH: u8 = 0x05;
    pub const TIMER_1_LATCH_LOW: u8 = 0x06;
    pub const TIMER_1_LATCH_HIGH: u8 = 0x07;
    pub const TIMER_2_COUNTER_LOW: u8 = 0x08;
    pub const TIMER_2_COUNTER_HIGH: u8 = 0x09;
    pub const SHIFT_REGISTER: u8 = 0x0a;
    pub const AUXILIARY_CONTROL: u8 = 0x0b;
    pub const PERIPHERAL_CONTROL: u8 = 0x0c;
    pub const INTERRUPT_FLAGS: u8 = 0x0d;
    pub const INTERRUPT_ENABLE: u8 = 0x0e;
    pub const PORT_A_WITHOUT_HANDSHAKE: u8 = 0x0f;
}

pub mod interrupt {
    pub const CA2: u8 = 1 << 0;
    pub const CA1: u8 = 1 << 1;
    pub const SHIFT_REGISTER: u8 = 1 << 2;
    pub const CB2: u8 = 1 << 3;
    pub const CB1: u8 = 1 << 4;
    pub const TIMER_2: u8 = 1 << 5;
    pub const TIMER_1: u8 = 1 << 6;
    pub const ANY: u8 = 1 << 7;
    pub const SOURCE_MASK: u8 = 0x7f;
}

pub mod auxiliary_control {
    pub const PORT_A_INPUT_LATCH: u8 = 1 << 0;
    pub const PORT_B_INPUT_LATCH: u8 = 1 << 1;
    pub const SHIFT_MODE_MASK: u8 = 0x1c;
    pub const TIMER_2_COUNT_PORT_B_6: u8 = 1 << 5;
    pub const TIMER_1_FREE_RUNNING: u8 = 1 << 6;
    pub const TIMER_1_PORT_B_7_OUTPUT: u8 = 1 << 7;
}

pub mod peripheral_control_mode {
    pub const INPUT_NEGATIVE_EDGE: u8 = 0;
    pub const INPUT_NEGATIVE_EDGE_INDEPENDENT: u8 = 1;
    pub const INPUT_POSITIVE_EDGE: u8 = 2;
    pub const INPUT_POSITIVE_EDGE_INDEPENDENT: u8 = 3;
    pub const HANDSHAKE_OUTPUT: u8 = 4;
    pub const PULSE_OUTPUT: u8 = 5;
    pub const LOW_OUTPUT: u8 = 6;
    pub const HIGH_OUTPUT: u8 = 7;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Mos6522ShiftMode {
    Disabled = 0,
    InputTimer2 = 1,
    InputProcessorClock = 2,
    InputExternalClock = 3,
    OutputFreeRunningTimer2 = 4,
    OutputTimer2 = 5,
    OutputProcessorClock = 6,
    OutputExternalClock = 7,
}

impl Mos6522ShiftMode {
    const fn from_control(value: u8) -> Self {
        match (value & auxiliary_control::SHIFT_MODE_MASK) >> 2 {
            0 => Self::Disabled,
            1 => Self::InputTimer2,
            2 => Self::InputProcessorClock,
            3 => Self::InputExternalClock,
            4 => Self::OutputFreeRunningTimer2,
            5 => Self::OutputTimer2,
            6 => Self::OutputProcessorClock,
            _ => Self::OutputExternalClock,
        }
    }

    const fn outputs_data(self) -> bool {
        self as u8 >= Self::OutputFreeRunningTimer2 as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mos6522ControlLine {
    Ca1,
    Ca2,
    Cb1,
    Cb2,
}

const STATE_TIMER_1_RUNNING: u16 = 1 << 0;
const STATE_TIMER_1_IRQ_ARMED: u16 = 1 << 1;
const STATE_TIMER_1_RELOAD_PENDING: u16 = 1 << 2;
const STATE_TIMER_1_PORT_B_7_HIGH: u16 = 1 << 3;
const STATE_TIMER_2_RUNNING: u16 = 1 << 4;
const STATE_TIMER_2_IRQ_ARMED: u16 = 1 << 5;
const STATE_PORT_B_6_HIGH: u16 = 1 << 6;
const STATE_CA1_HIGH: u16 = 1 << 7;
const STATE_CA2_INPUT_HIGH: u16 = 1 << 8;
const STATE_CB1_HIGH: u16 = 1 << 9;
const STATE_CB2_INPUT_HIGH: u16 = 1 << 10;
const STATE_CA2_OUTPUT_HIGH: u16 = 1 << 11;
const STATE_CB2_OUTPUT_HIGH: u16 = 1 << 12;
const STATE_CB1_OUTPUT_HIGH: u16 = 1 << 13;

/// MOS 6522 register, timer, shift and handshake core. Board wrappers update
/// external input pins before accesses and consume the exposed output levels.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Mos6522 {
    registers: [u8; MOS6522_REGISTER_COUNT],
    timer_1_counter: u16,
    timer_1_latch: u16,
    timer_1_start_delay: u8,
    timer_2_counter: u16,
    timer_2_low_latch: u8,
    timer_2_start_delay: u8,
    interrupt_flags: u8,
    interrupt_enable: u8,
    latched_port_a: u8,
    latched_port_b: u8,
    external_port_a: u8,
    external_port_b: u8,
    shift_phase: u8,
    shift_start_delay: u8,
    ca2_pulse_cycles_remaining: u8,
    cb2_pulse_cycles_remaining: u8,
    state_flags: u16,
}

impl Default for Mos6522 {
    fn default() -> Self {
        Self::new()
    }
}

impl Mos6522 {
    pub fn new() -> Self {
        let mut result = Self {
            registers: [0; MOS6522_REGISTER_COUNT],
            timer_1_counter: u16::MAX,
            timer_1_latch: u16::MAX,
            timer_1_start_delay: 0,
            timer_2_counter: u16::MAX,
            timer_2_low_latch: u8::MAX,
            timer_2_start_delay: 0,
            interrupt_flags: 0,
            interrupt_enable: 0,
            latched_port_a: u8::MAX,
            latched_port_b: u8::MAX,
            external_port_a: u8::MAX,
            external_port_b: u8::MAX,
            shift_phase: FINISHED_SHIFT_PHASE,
            shift_start_delay: 0,
            ca2_pulse_cycles_remaining: 0,
            cb2_pulse_cycles_remaining: 0,
            state_flags: 0,
        };
        result.reset();
        result
    }

    pub const fn interrupt_pending(&self) -> bool {
        self.interrupt_flags & self.interrupt_enable & interrupt::SOURCE_MASK != 0
    }

    pub const fn interrupt_flags(&self) -> u8 {
        self.interrupt_flags
    }

    pub const fn interrupt_enable(&self) -> u8 {
        self.interrupt_enable
    }

    pub const fn timer_1_counter(&self) -> u16 {
        self.timer_1_counter
    }

    pub const fn timer_2_counter(&self) -> u16 {
        self.timer_2_counter
    }

    pub const fn shift_phase(&self) -> u8 {
        self.shift_phase
    }

    pub const fn port_a_output_pins(&self) -> u8 {
        let output = self.registers[register::PORT_A as usize];
        let direction = self.registers[register::DATA_DIRECTION_A as usize];
        (output & direction) | !direction
    }

    pub const fn port_b_output_latch(&self) -> u8 {
        self.registers[register::PORT_B as usize]
    }

    pub const fn port_b_data_direction(&self) -> u8 {
        self.registers[register::DATA_DIRECTION_B as usize]
    }

    pub const fn port_b_output_pins(&self) -> u8 {
        let output = self.port_b_output_latch();
        let direction = self.port_b_data_direction();
        let mut pins = (output & direction) | !direction;
        if self.timer_1_controls_port_b_7() {
            if self.state_flag(STATE_TIMER_1_PORT_B_7_HIGH) {
                pins |= PORT_B_7_BIT;
            } else {
                pins &= !PORT_B_7_BIT;
            }
        }
        pins
    }

    pub const fn ca2_output_high(&self) -> bool {
        self.state_flag(STATE_CA2_OUTPUT_HIGH)
    }

    pub const fn cb1_output_high(&self) -> bool {
        self.state_flag(STATE_CB1_OUTPUT_HIGH)
    }

    pub const fn cb2_output_high(&self) -> bool {
        self.state_flag(STATE_CB2_OUTPUT_HIGH)
    }

    pub fn set_port_a_external_inputs(&mut self, value: u8) {
        self.external_port_a = value;
    }

    pub fn set_port_b_external_inputs(&mut self, value: u8) {
        self.external_port_b = value;
    }

    pub fn reset(&mut self) {
        self.registers.fill(0);
        self.timer_1_counter = u16::MAX;
        self.timer_1_latch = u16::MAX;
        self.timer_1_start_delay = 0;
        self.timer_2_counter = u16::MAX;
        self.timer_2_low_latch = u8::MAX;
        self.timer_2_start_delay = 0;
        self.interrupt_flags = 0;
        self.interrupt_enable = 0;
        self.latched_port_a = u8::MAX;
        self.latched_port_b = u8::MAX;
        self.shift_phase = FINISHED_SHIFT_PHASE;
        self.shift_start_delay = 0;
        self.ca2_pulse_cycles_remaining = 0;
        self.cb2_pulse_cycles_remaining = 0;
        self.state_flags = STATE_TIMER_1_PORT_B_7_HIGH
            | STATE_PORT_B_6_HIGH
            | STATE_CA1_HIGH
            | STATE_CA2_INPUT_HIGH
            | STATE_CB1_HIGH
            | STATE_CB2_INPUT_HIGH
            | STATE_CA2_OUTPUT_HIGH
            | STATE_CB1_OUTPUT_HIGH
            | STATE_CB2_OUTPUT_HIGH;
    }

    pub fn clock_cycles(&mut self, cycles: u32) -> bool {
        for _ in 0..cycles {
            self.clock_cycle();
        }
        self.interrupt_pending()
    }

    pub fn clock_cycle(&mut self) -> bool {
        self.clock_timer_1();
        self.clock_timer_2();
        self.clock_processor_shift();
        self.clock_control_line_pulses();
        self.interrupt_pending()
    }

    pub fn read(&mut self, address: u16) -> u8 {
        let index = address.to_le_bytes()[0] & 0x0f;
        match index {
            register::PORT_B => self.read_port_b(true),
            register::PORT_A => self.read_port_a(true),
            register::DATA_DIRECTION_B
            | register::DATA_DIRECTION_A
            | register::AUXILIARY_CONTROL
            | register::PERIPHERAL_CONTROL => self.registers[usize::from(index)],
            register::TIMER_1_COUNTER_LOW => {
                self.clear_interrupt(interrupt::TIMER_1);
                self.timer_1_counter.to_le_bytes()[0]
            }
            register::TIMER_1_COUNTER_HIGH => self.timer_1_counter.to_le_bytes()[1],
            register::TIMER_1_LATCH_LOW => self.timer_1_latch.to_le_bytes()[0],
            register::TIMER_1_LATCH_HIGH => self.timer_1_latch.to_le_bytes()[1],
            register::TIMER_2_COUNTER_LOW => {
                self.clear_interrupt(interrupt::TIMER_2);
                self.timer_2_counter.to_le_bytes()[0]
            }
            register::TIMER_2_COUNTER_HIGH => self.timer_2_counter.to_le_bytes()[1],
            register::SHIFT_REGISTER => {
                self.clear_interrupt(interrupt::SHIFT_REGISTER);
                self.start_shift_transfer();
                self.registers[register::SHIFT_REGISTER as usize]
            }
            register::INTERRUPT_FLAGS => {
                self.interrupt_flags
                    | if self.interrupt_pending() {
                        interrupt::ANY
                    } else {
                        0
                    }
            }
            register::INTERRUPT_ENABLE => self.interrupt_enable | interrupt::ANY,
            register::PORT_A_WITHOUT_HANDSHAKE => self.read_port_a(false),
            _ => 0,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        let index = address.to_le_bytes()[0] & 0x0f;
        match index {
            register::PORT_B => self.write_port_b(value, true),
            register::PORT_A => self.write_port_a(value, true),
            register::DATA_DIRECTION_B | register::DATA_DIRECTION_A => {
                self.registers[usize::from(index)] = value;
            }
            register::TIMER_1_COUNTER_LOW | register::TIMER_1_LATCH_LOW => {
                self.timer_1_latch = (self.timer_1_latch & 0xff00) | u16::from(value);
            }
            register::TIMER_1_COUNTER_HIGH => self.start_timer_1(value),
            register::TIMER_1_LATCH_HIGH => {
                self.timer_1_latch = (self.timer_1_latch & 0x00ff) | (u16::from(value) << 8);
                self.clear_interrupt(interrupt::TIMER_1);
            }
            register::TIMER_2_COUNTER_LOW => self.timer_2_low_latch = value,
            register::TIMER_2_COUNTER_HIGH => self.start_timer_2(value),
            register::SHIFT_REGISTER => {
                self.registers[usize::from(index)] = value;
                self.clear_interrupt(interrupt::SHIFT_REGISTER);
                self.start_shift_transfer();
            }
            register::AUXILIARY_CONTROL => self.write_auxiliary_control(value),
            register::PERIPHERAL_CONTROL => self.write_peripheral_control(value),
            register::INTERRUPT_FLAGS => {
                self.interrupt_flags &= !(value & interrupt::SOURCE_MASK);
            }
            register::INTERRUPT_ENABLE => {
                let selected = value & interrupt::SOURCE_MASK;
                if value & interrupt::ANY != 0 {
                    self.interrupt_enable |= selected;
                } else {
                    self.interrupt_enable &= !selected;
                }
            }
            register::PORT_A_WITHOUT_HANDSHAKE => self.write_port_a(value, false),
            _ => {}
        }
    }

    pub fn signal_control_line(&mut self, line: Mos6522ControlLine, high: bool) {
        match line {
            Mos6522ControlLine::Ca1 => {
                let previous = self.state_flag(STATE_CA1_HIGH);
                self.set_state_flag(STATE_CA1_HIGH, high);
                if previous != high && self.is_ca1_active_edge(previous, high) {
                    self.handle_ca1_edge();
                }
            }
            Mos6522ControlLine::Ca2 => {
                let previous = self.state_flag(STATE_CA2_INPUT_HIGH);
                self.set_state_flag(STATE_CA2_INPUT_HIGH, high);
                if previous != high
                    && self.is_ca2_input_mode()
                    && Self::is_control_mode_active_edge(self.ca2_control_mode(), previous, high)
                {
                    self.raise_interrupt(interrupt::CA2);
                }
            }
            Mos6522ControlLine::Cb1 => {
                let previous = self.state_flag(STATE_CB1_HIGH);
                self.set_state_flag(STATE_CB1_HIGH, high);
                if previous != high {
                    self.handle_external_shift_clock(high);
                    if self.is_cb1_active_edge(previous, high) {
                        self.handle_cb1_edge();
                    }
                }
            }
            Mos6522ControlLine::Cb2 => {
                let previous = self.state_flag(STATE_CB2_INPUT_HIGH);
                self.set_state_flag(STATE_CB2_INPUT_HIGH, high);
                if previous != high
                    && self.is_cb2_input_mode()
                    && Self::is_control_mode_active_edge(self.cb2_control_mode(), previous, high)
                {
                    self.raise_interrupt(interrupt::CB2);
                }
            }
        }
    }

    pub fn signal_port_b_6(&mut self, high: bool) {
        let falling_edge = self.state_flag(STATE_PORT_B_6_HIGH) && !high;
        self.set_state_flag(STATE_PORT_B_6_HIGH, high);
        if falling_edge && self.timer_2_counts_port_b_6() {
            self.step_timer_2_counter();
        }
    }

    fn read_port_a(&mut self, handshake: bool) -> u8 {
        if handshake {
            self.handle_port_a_handshake();
        }
        let direction = self.registers[register::DATA_DIRECTION_A as usize];
        let output = self.registers[register::PORT_A as usize];
        let external = if self.registers[register::AUXILIARY_CONTROL as usize]
            & auxiliary_control::PORT_A_INPUT_LATCH
            != 0
        {
            self.latched_port_a
        } else {
            self.external_port_a
        };
        (external & !direction) | (output & direction)
    }

    fn read_port_b(&mut self, handshake: bool) -> u8 {
        if handshake {
            self.handle_port_b_handshake();
        }
        let direction = self.registers[register::DATA_DIRECTION_B as usize];
        let output = self.registers[register::PORT_B as usize];
        let external = if self.registers[register::AUXILIARY_CONTROL as usize]
            & auxiliary_control::PORT_B_INPUT_LATCH
            != 0
        {
            self.latched_port_b
        } else {
            self.external_port_b
        };
        let mut value = (external & !direction) | (output & direction);
        if self.timer_1_controls_port_b_7() {
            if self.state_flag(STATE_TIMER_1_PORT_B_7_HIGH) {
                value |= PORT_B_7_BIT;
            } else {
                value &= !PORT_B_7_BIT;
            }
        }
        value
    }

    fn write_port_a(&mut self, value: u8, handshake: bool) {
        if handshake {
            self.handle_port_a_handshake();
        }
        self.registers[register::PORT_A as usize] = value;
        self.registers[register::PORT_A_WITHOUT_HANDSHAKE as usize] = value;
    }

    fn write_port_b(&mut self, value: u8, handshake: bool) {
        if handshake {
            self.handle_port_b_handshake();
        }
        self.registers[register::PORT_B as usize] = value;
    }

    fn handle_port_a_handshake(&mut self) {
        self.clear_interrupt(interrupt::CA1);
        if !self.ca2_interrupt_independent() {
            self.clear_interrupt(interrupt::CA2);
        }
        let mode = self.ca2_control_mode();
        if mode == peripheral_control_mode::HANDSHAKE_OUTPUT
            || mode == peripheral_control_mode::PULSE_OUTPUT
        {
            self.set_ca2_output(false);
            if mode == peripheral_control_mode::PULSE_OUTPUT {
                self.ca2_pulse_cycles_remaining = 1;
            }
        }
    }

    fn handle_port_b_handshake(&mut self) {
        self.clear_interrupt(interrupt::CB1);
        if !self.cb2_interrupt_independent() {
            self.clear_interrupt(interrupt::CB2);
        }
        let mode = self.cb2_control_mode();
        if mode == peripheral_control_mode::HANDSHAKE_OUTPUT
            || mode == peripheral_control_mode::PULSE_OUTPUT
        {
            self.set_cb2_output(false);
            if mode == peripheral_control_mode::PULSE_OUTPUT {
                self.cb2_pulse_cycles_remaining = 1;
            }
        }
    }

    fn start_timer_1(&mut self, high_byte: u8) {
        self.timer_1_latch = (self.timer_1_latch & 0x00ff) | (u16::from(high_byte) << 8);
        self.timer_1_counter = self.timer_1_latch;
        self.set_state_flag(STATE_TIMER_1_RUNNING, true);
        self.set_state_flag(STATE_TIMER_1_IRQ_ARMED, true);
        self.set_state_flag(STATE_TIMER_1_RELOAD_PENDING, false);
        self.timer_1_start_delay = 1;
        self.set_state_flag(STATE_TIMER_1_PORT_B_7_HIGH, false);
        self.clear_interrupt(interrupt::TIMER_1);
    }

    fn start_timer_2(&mut self, high_byte: u8) {
        self.timer_2_counter = (u16::from(high_byte) << 8) | u16::from(self.timer_2_low_latch);
        self.set_state_flag(STATE_TIMER_2_RUNNING, true);
        self.set_state_flag(STATE_TIMER_2_IRQ_ARMED, true);
        self.timer_2_start_delay = 1;
        self.clear_interrupt(interrupt::TIMER_2);
    }

    fn clock_timer_1(&mut self) {
        if !self.state_flag(STATE_TIMER_1_RUNNING) {
            return;
        }
        if self.timer_1_start_delay > 0 {
            self.timer_1_start_delay -= 1;
            return;
        }
        if self.state_flag(STATE_TIMER_1_RELOAD_PENDING) {
            self.timer_1_counter = self.timer_1_latch;
            self.set_state_flag(STATE_TIMER_1_RELOAD_PENDING, false);
            return;
        }
        if self.timer_1_counter != 0 {
            self.timer_1_counter = self.timer_1_counter.wrapping_sub(1);
            return;
        }

        self.timer_1_counter = u16::MAX;
        self.set_state_flag(STATE_TIMER_1_RELOAD_PENDING, true);
        let timeout_armed = self.state_flag(STATE_TIMER_1_IRQ_ARMED);
        if timeout_armed {
            self.raise_interrupt(interrupt::TIMER_1);
        }
        self.set_state_flag(
            STATE_TIMER_1_IRQ_ARMED,
            timeout_armed && self.timer_1_free_running(),
        );
        if timeout_armed && self.timer_1_controls_port_b_7() {
            let high = !self.state_flag(STATE_TIMER_1_PORT_B_7_HIGH);
            self.set_state_flag(STATE_TIMER_1_PORT_B_7_HIGH, high);
        }
    }

    fn clock_timer_2(&mut self) {
        if !self.state_flag(STATE_TIMER_2_RUNNING) || self.timer_2_counts_port_b_6() {
            return;
        }
        if self.timer_2_start_delay > 0 {
            self.timer_2_start_delay -= 1;
            return;
        }
        self.step_timer_2_counter();
    }

    fn step_timer_2_counter(&mut self) {
        if !self.state_flag(STATE_TIMER_2_RUNNING) {
            return;
        }
        let previous = self.timer_2_counter;
        self.timer_2_counter = previous.wrapping_sub(1);
        if previous.to_le_bytes()[0] == 0 && self.shift_uses_timer_2() {
            self.timer_2_counter =
                (self.timer_2_counter & 0xff00) | u16::from(self.timer_2_low_latch);
            self.advance_shift_phase(true);
        }
        if previous == 0 && self.state_flag(STATE_TIMER_2_IRQ_ARMED) {
            self.set_state_flag(STATE_TIMER_2_IRQ_ARMED, false);
            self.raise_interrupt(interrupt::TIMER_2);
        }
    }

    fn write_auxiliary_control(&mut self, value: u8) {
        let previous = self.registers[register::AUXILIARY_CONTROL as usize];
        self.registers[register::AUXILIARY_CONTROL as usize] = value;
        if previous & auxiliary_control::PORT_A_INPUT_LATCH == 0
            && value & auxiliary_control::PORT_A_INPUT_LATCH != 0
        {
            self.latched_port_a = self.external_port_a;
        }
        if previous & auxiliary_control::PORT_B_INPUT_LATCH == 0
            && value & auxiliary_control::PORT_B_INPUT_LATCH != 0
        {
            self.latched_port_b = self.external_port_b;
        }
        if previous & auxiliary_control::TIMER_1_PORT_B_7_OUTPUT == 0
            && value & auxiliary_control::TIMER_1_PORT_B_7_OUTPUT != 0
        {
            self.set_state_flag(STATE_TIMER_1_PORT_B_7_HIGH, true);
        }
        if self.shift_mode() == Mos6522ShiftMode::Disabled {
            self.clear_interrupt(interrupt::SHIFT_REGISTER);
            self.update_cb2_from_peripheral_control();
        }
    }

    fn write_peripheral_control(&mut self, value: u8) {
        self.registers[register::PERIPHERAL_CONTROL as usize] = value;
        self.ca2_pulse_cycles_remaining = 0;
        self.cb2_pulse_cycles_remaining = 0;
        self.update_ca2_from_peripheral_control();
        self.update_cb2_from_peripheral_control();
    }

    fn update_ca2_from_peripheral_control(&mut self) {
        self.set_ca2_output(self.ca2_control_mode() != peripheral_control_mode::LOW_OUTPUT);
    }

    fn update_cb2_from_peripheral_control(&mut self) {
        if !self.shift_mode().outputs_data() {
            self.set_cb2_output(self.cb2_control_mode() != peripheral_control_mode::LOW_OUTPUT);
        }
    }

    fn handle_ca1_edge(&mut self) {
        if self.registers[register::AUXILIARY_CONTROL as usize]
            & auxiliary_control::PORT_A_INPUT_LATCH
            != 0
        {
            self.latched_port_a = self.external_port_a;
        }
        self.raise_interrupt(interrupt::CA1);
        if self.ca2_control_mode() == peripheral_control_mode::HANDSHAKE_OUTPUT {
            self.set_ca2_output(true);
        }
    }

    fn handle_cb1_edge(&mut self) {
        if self.registers[register::AUXILIARY_CONTROL as usize]
            & auxiliary_control::PORT_B_INPUT_LATCH
            != 0
        {
            self.latched_port_b = self.external_port_b;
        }
        self.raise_interrupt(interrupt::CB1);
        if self.cb2_control_mode() == peripheral_control_mode::HANDSHAKE_OUTPUT {
            self.set_cb2_output(true);
        }
    }

    fn start_shift_transfer(&mut self) {
        let mode = self.shift_mode();
        if mode == Mos6522ShiftMode::Disabled {
            return;
        }
        if mode == Mos6522ShiftMode::OutputFreeRunningTimer2 {
            self.shift_phase &= FINISHED_SHIFT_PHASE - 1;
        } else if self.shift_phase == FINISHED_SHIFT_PHASE {
            self.shift_phase = 0;
        }
        if mode == Mos6522ShiftMode::InputProcessorClock
            || mode == Mos6522ShiftMode::OutputProcessorClock
        {
            self.shift_start_delay = 1;
        }
    }

    fn clock_processor_shift(&mut self) {
        let mode = self.shift_mode();
        if mode != Mos6522ShiftMode::InputProcessorClock
            && mode != Mos6522ShiftMode::OutputProcessorClock
        {
            return;
        }
        if self.shift_phase >= FINISHED_SHIFT_PHASE {
            return;
        }
        if self.shift_start_delay > 0 {
            self.shift_start_delay -= 1;
            return;
        }
        self.advance_shift_phase(true);
    }

    fn handle_external_shift_clock(&mut self, high: bool) {
        let mode = self.shift_mode();
        if mode != Mos6522ShiftMode::InputExternalClock
            && mode != Mos6522ShiftMode::OutputExternalClock
        {
            return;
        }
        if self.shift_phase >= FINISHED_SHIFT_PHASE {
            return;
        }
        let expected_high = self.shift_phase & 1 != 0;
        if high == expected_high {
            self.advance_shift_phase(false);
        }
    }

    fn advance_shift_phase(&mut self, drive_clock_output: bool) {
        if self.shift_phase >= FINISHED_SHIFT_PHASE {
            return;
        }
        let output = self.shift_mode().outputs_data();
        let even_phase = self.shift_phase & 1 == 0;
        if drive_clock_output {
            self.set_state_flag(STATE_CB1_OUTPUT_HIGH, !even_phase);
        }
        let shift_register = self.registers[register::SHIFT_REGISTER as usize];
        if even_phase && output {
            let output_high = shift_register & 0x80 != 0;
            self.registers[register::SHIFT_REGISTER as usize] = shift_register.rotate_left(1);
            self.set_cb2_output(output_high);
        } else if !even_phase && !output {
            self.registers[register::SHIFT_REGISTER as usize] =
                (shift_register << 1) | u8::from(self.state_flag(STATE_CB2_INPUT_HIGH));
        }
        self.shift_phase += 1;
        if self.shift_phase == FINISHED_SHIFT_PHASE {
            if self.shift_mode() == Mos6522ShiftMode::OutputFreeRunningTimer2 {
                self.shift_phase = 0;
            } else {
                self.raise_interrupt(interrupt::SHIFT_REGISTER);
            }
        }
    }

    fn clock_control_line_pulses(&mut self) {
        if self.ca2_pulse_cycles_remaining > 0 {
            self.ca2_pulse_cycles_remaining -= 1;
            if self.ca2_pulse_cycles_remaining == 0 {
                self.set_ca2_output(true);
            }
        }
        if self.cb2_pulse_cycles_remaining > 0 {
            self.cb2_pulse_cycles_remaining -= 1;
            if self.cb2_pulse_cycles_remaining == 0 {
                self.set_cb2_output(true);
            }
        }
    }

    fn set_ca2_output(&mut self, high: bool) {
        self.set_state_flag(STATE_CA2_OUTPUT_HIGH, high);
    }

    fn set_cb2_output(&mut self, high: bool) {
        self.set_state_flag(STATE_CB2_OUTPUT_HIGH, high);
    }

    fn raise_interrupt(&mut self, mask: u8) {
        self.interrupt_flags |= mask & interrupt::SOURCE_MASK;
    }

    fn clear_interrupt(&mut self, mask: u8) {
        self.interrupt_flags &= !mask;
    }

    const fn timer_1_free_running(&self) -> bool {
        self.registers[register::AUXILIARY_CONTROL as usize]
            & auxiliary_control::TIMER_1_FREE_RUNNING
            != 0
    }

    const fn timer_1_controls_port_b_7(&self) -> bool {
        self.registers[register::AUXILIARY_CONTROL as usize]
            & auxiliary_control::TIMER_1_PORT_B_7_OUTPUT
            != 0
    }

    const fn timer_2_counts_port_b_6(&self) -> bool {
        self.registers[register::AUXILIARY_CONTROL as usize]
            & auxiliary_control::TIMER_2_COUNT_PORT_B_6
            != 0
    }

    const fn shift_mode(&self) -> Mos6522ShiftMode {
        Mos6522ShiftMode::from_control(self.registers[register::AUXILIARY_CONTROL as usize])
    }

    const fn shift_uses_timer_2(&self) -> bool {
        matches!(
            self.shift_mode(),
            Mos6522ShiftMode::InputTimer2
                | Mos6522ShiftMode::OutputFreeRunningTimer2
                | Mos6522ShiftMode::OutputTimer2
        )
    }

    const fn ca2_control_mode(&self) -> u8 {
        (self.registers[register::PERIPHERAL_CONTROL as usize] >> 1) & 0x07
    }

    const fn cb2_control_mode(&self) -> u8 {
        (self.registers[register::PERIPHERAL_CONTROL as usize] >> 5) & 0x07
    }

    const fn is_ca2_input_mode(&self) -> bool {
        self.ca2_control_mode() <= peripheral_control_mode::INPUT_POSITIVE_EDGE_INDEPENDENT
    }

    const fn is_cb2_input_mode(&self) -> bool {
        self.cb2_control_mode() <= peripheral_control_mode::INPUT_POSITIVE_EDGE_INDEPENDENT
    }

    const fn ca2_interrupt_independent(&self) -> bool {
        let mode = self.ca2_control_mode();
        mode == peripheral_control_mode::INPUT_NEGATIVE_EDGE_INDEPENDENT
            || mode == peripheral_control_mode::INPUT_POSITIVE_EDGE_INDEPENDENT
    }

    const fn cb2_interrupt_independent(&self) -> bool {
        let mode = self.cb2_control_mode();
        mode == peripheral_control_mode::INPUT_NEGATIVE_EDGE_INDEPENDENT
            || mode == peripheral_control_mode::INPUT_POSITIVE_EDGE_INDEPENDENT
    }

    const fn is_ca1_active_edge(&self, previous: bool, high: bool) -> bool {
        let positive = self.registers[register::PERIPHERAL_CONTROL as usize] & 0x01 != 0;
        if positive {
            !previous && high
        } else {
            previous && !high
        }
    }

    const fn is_cb1_active_edge(&self, previous: bool, high: bool) -> bool {
        let positive = self.registers[register::PERIPHERAL_CONTROL as usize] & 0x10 != 0;
        if positive {
            !previous && high
        } else {
            previous && !high
        }
    }

    const fn is_control_mode_active_edge(mode: u8, previous: bool, high: bool) -> bool {
        let positive = mode == peripheral_control_mode::INPUT_POSITIVE_EDGE
            || mode == peripheral_control_mode::INPUT_POSITIVE_EDGE_INDEPENDENT;
        if positive {
            !previous && high
        } else {
            previous && !high
        }
    }

    const fn state_flag(&self, flag: u16) -> bool {
        self.state_flags & flag != 0
    }

    fn set_state_flag(&mut self, flag: u16, enabled: bool) {
        if enabled {
            self.state_flags |= flag;
        } else {
            self.state_flags &= !flag;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Mos6522, Mos6522ControlLine, auxiliary_control, interrupt, register};

    #[test]
    fn combines_output_direction_and_external_input_pins() {
        let mut via = Mos6522::new();
        via.set_port_a_external_inputs(0xa0);
        via.write(u16::from(register::DATA_DIRECTION_A), 0x0f);
        via.write(u16::from(register::PORT_A), 0x05);

        assert_eq!(via.port_a_output_pins(), 0xf5);
        assert_eq!(via.read(u16::from(register::PORT_A)), 0xa5);
    }

    #[test]
    fn timer_1_observes_start_zero_and_underflow_cycles() {
        let mut via = Mos6522::new();
        via.write(
            u16::from(register::INTERRUPT_ENABLE),
            interrupt::ANY | interrupt::TIMER_1,
        );
        via.write(u16::from(register::TIMER_1_COUNTER_LOW), 2);
        via.write(u16::from(register::TIMER_1_COUNTER_HIGH), 0);

        via.clock_cycle();
        assert_eq!(via.timer_1_counter(), 2);
        via.clock_cycles(2);
        assert!(!via.interrupt_pending());
        via.clock_cycle();
        assert!(via.interrupt_pending());
        assert_eq!(
            via.read(u16::from(register::INTERRUPT_FLAGS)),
            interrupt::ANY | interrupt::TIMER_1
        );
        via.read(u16::from(register::TIMER_1_COUNTER_LOW));
        assert!(!via.interrupt_pending());
    }

    #[test]
    fn free_running_timer_1_toggles_port_b_7() {
        let mut via = Mos6522::new();
        via.write(u16::from(register::DATA_DIRECTION_B), 0x80);
        via.write(
            u16::from(register::AUXILIARY_CONTROL),
            auxiliary_control::TIMER_1_FREE_RUNNING | auxiliary_control::TIMER_1_PORT_B_7_OUTPUT,
        );
        via.write(u16::from(register::TIMER_1_COUNTER_LOW), 0);
        via.write(u16::from(register::TIMER_1_COUNTER_HIGH), 0);
        assert_eq!(via.port_b_output_pins() & 0x80, 0);
        via.clock_cycles(2);
        assert_eq!(via.port_b_output_pins() & 0x80, 0x80);
        via.clock_cycles(2);
        assert_eq!(via.port_b_output_pins() & 0x80, 0);
    }

    #[test]
    fn timer_2_counts_processor_cycles_or_port_b_6_edges() {
        let mut via = Mos6522::new();
        via.write(u16::from(register::TIMER_2_COUNTER_LOW), 1);
        via.write(u16::from(register::TIMER_2_COUNTER_HIGH), 0);
        via.clock_cycles(3);
        assert_ne!(via.interrupt_flags() & interrupt::TIMER_2, 0);

        let mut pulse = Mos6522::new();
        pulse.write(
            u16::from(register::AUXILIARY_CONTROL),
            auxiliary_control::TIMER_2_COUNT_PORT_B_6,
        );
        pulse.write(u16::from(register::TIMER_2_COUNTER_LOW), 1);
        pulse.write(u16::from(register::TIMER_2_COUNTER_HIGH), 0);
        pulse.clock_cycles(100);
        assert_eq!(pulse.interrupt_flags() & interrupt::TIMER_2, 0);
        pulse.signal_port_b_6(false);
        pulse.signal_port_b_6(true);
        pulse.signal_port_b_6(false);
        assert_ne!(pulse.interrupt_flags() & interrupt::TIMER_2, 0);
    }

    #[test]
    fn processor_clock_shift_raises_interrupt_after_sixteen_phases() {
        let mut via = Mos6522::new();
        via.write(
            u16::from(register::AUXILIARY_CONTROL),
            (super::Mos6522ShiftMode::OutputProcessorClock as u8) << 2,
        );
        via.write(u16::from(register::SHIFT_REGISTER), 0xa5);
        via.clock_cycles(17);

        assert_ne!(via.interrupt_flags() & interrupt::SHIFT_REGISTER, 0);
        assert_eq!(via.read(u16::from(register::SHIFT_REGISTER)), 0xa5);
    }

    #[test]
    fn ca1_latches_port_a_and_releases_handshake_output() {
        let mut via = Mos6522::new();
        via.set_port_a_external_inputs(0xaa);
        via.write(
            u16::from(register::AUXILIARY_CONTROL),
            auxiliary_control::PORT_A_INPUT_LATCH,
        );
        via.write(
            u16::from(register::PERIPHERAL_CONTROL),
            super::peripheral_control_mode::HANDSHAKE_OUTPUT << 1,
        );
        via.write(u16::from(register::PORT_A), 0);
        assert!(!via.ca2_output_high());
        via.signal_control_line(Mos6522ControlLine::Ca1, false);
        assert!(via.ca2_output_high());
        via.set_port_a_external_inputs(0x55);
        assert_eq!(
            via.read(u16::from(register::PORT_A_WITHOUT_HANDSHAKE)),
            0xaa
        );
    }
}
