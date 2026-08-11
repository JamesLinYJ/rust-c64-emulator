// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - MOS 6526 timer signal pipeline
//
//   File:       timer.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use super::{CONTROL_FORCE_LOAD, CONTROL_ONE_SHOT, CONTROL_START, CONTROL_TOGGLE_OUTPUT};

const STATE_START: u16 = 0x0001;
const STATE_COUNT_STAGE_2: u16 = 0x0002;
const STATE_EXTERNAL_STEP: u16 = 0x0004;
const STATE_ONE_SHOT_CONTROL: u16 = 0x0008;
const STATE_FORCE_LOAD_CONTROL: u16 = 0x0010;
const STATE_PROCESSOR_CLOCK_INPUT: u16 = 0x0020;
const STATE_COUNT_STAGE_3: u16 = 0x0040;
const STATE_LOAD_STAGE_1: u16 = 0x0080;
const STATE_ONE_SHOT_STAGE_0: u16 = 0x0100;
const STATE_LOAD: u16 = 0x0200;
const STATE_OUTPUT: u16 = 0x0400;
const STATE_COUNT: u16 = 0x0800;
const STATE_ONE_SHOT: u16 = 0x1000;
const CONTROL_STATE_MASK: u16 =
    STATE_START | STATE_ONE_SHOT_CONTROL | STATE_FORCE_LOAD_CONTROL | STATE_PROCESSOR_CLOCK_INPUT;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum TimerInputMode {
    #[default]
    ProcessorClock,
    CountPin,
    TimerAUnderflow,
    TimerAUnderflowWhileCountHigh,
}

impl TimerInputMode {
    pub(super) const fn from_timer_b_control(control: u8) -> Self {
        match (control >> 5) & 0x03 {
            0 => Self::ProcessorClock,
            1 => Self::CountPin,
            2 => Self::TimerAUnderflow,
            _ => Self::TimerAUnderflowWhileCountHigh,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Mos6526Timer {
    counter: u16,
    pub(super) input_mode: TimerInputMode,
    latch: u16,
    toggle_output: bool,
    state: u16,
    underflow_pulse_active: bool,
    toggle_state_high: bool,
}

impl Default for Mos6526Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mos6526Timer {
    pub(super) const fn new() -> Self {
        Self {
            counter: u16::MAX,
            input_mode: TimerInputMode::ProcessorClock,
            latch: u16::MAX,
            toggle_output: false,
            state: 0,
            underflow_pulse_active: false,
            toggle_state_high: false,
        }
    }

    pub(super) const fn counter(self) -> u16 {
        self.counter
    }

    pub(super) const fn running(self) -> bool {
        self.state & STATE_START != 0
    }

    pub(super) const fn output_high(self) -> bool {
        if self.toggle_output {
            self.toggle_state_high
        } else {
            self.underflow_pulse_active
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(super) fn write_latch_low(&mut self, value: u8) {
        self.latch = (self.latch & 0xff00) | u16::from(value);
        if self.state & STATE_LOAD != 0 {
            self.counter = (self.counter & 0xff00) | u16::from(value);
        }
    }

    pub(super) fn write_latch_high(&mut self, value: u8) {
        self.latch = (u16::from(value) << 8) | (self.latch & 0x00ff);
        if self.state & STATE_LOAD != 0 || !self.running() {
            self.counter = self.latch;
        }
    }

    pub(super) fn write_control(&mut self, value: u8, input_mode: TimerInputMode) {
        let was_running = self.running();
        self.input_mode = input_mode;
        self.toggle_output = value & CONTROL_TOGGLE_OUTPUT != 0;
        self.state &= !CONTROL_STATE_MASK;
        if value & CONTROL_START != 0 {
            self.state |= STATE_START;
        }
        if !was_running && self.running() {
            self.toggle_state_high = true;
        }
        if value & CONTROL_ONE_SHOT != 0 {
            self.state |= STATE_ONE_SHOT_CONTROL;
        }
        if value & CONTROL_FORCE_LOAD != 0 {
            self.state |= STATE_FORCE_LOAD_CONTROL;
        }
        if input_mode == TimerInputMode::ProcessorClock {
            self.state |= STATE_PROCESSOR_CLOCK_INPUT;
        }
    }

    pub(super) fn tick_cycle(&mut self, external_step: bool) -> bool {
        self.underflow_pulse_active = false;
        if external_step && self.running() {
            self.state |= STATE_EXTERNAL_STEP;
        }

        if self.counter != 0 && self.state & STATE_COUNT_STAGE_3 != 0 {
            self.counter = self.counter.wrapping_sub(1);
        }
        self.state = Self::next_state(self.state);

        let underflow = self.counter == 0 && self.state & STATE_COUNT_STAGE_3 != 0;
        if underflow {
            self.state |= STATE_LOAD | STATE_OUTPUT;
        }
        if self.state & STATE_LOAD != 0 {
            self.counter = self.latch;
            self.state &= !STATE_COUNT_STAGE_3;
        }
        if self.state & STATE_OUTPUT != 0
            && self.state & (STATE_ONE_SHOT | STATE_ONE_SHOT_STAGE_0) != 0
        {
            self.state &= !(STATE_START | STATE_COUNT_STAGE_2);
        }
        if underflow {
            self.toggle_state_high = !self.toggle_state_high;
            self.underflow_pulse_active = true;
        }
        underflow
    }

    pub(super) fn schedule_external_step(&mut self) {
        if self.running() {
            self.state |= STATE_EXTERNAL_STEP;
        }
    }

    const fn next_state(state: u16) -> u16 {
        let mut next = state & (STATE_START | STATE_ONE_SHOT_CONTROL | STATE_PROCESSOR_CLOCK_INPUT);
        if state & STATE_START != 0 && state & STATE_PROCESSOR_CLOCK_INPUT != 0 {
            next |= STATE_COUNT_STAGE_2;
        }
        if state & STATE_COUNT_STAGE_2 != 0
            || state & STATE_EXTERNAL_STEP != 0 && state & STATE_START != 0
        {
            next |= STATE_COUNT_STAGE_3;
        }
        if state & STATE_COUNT_STAGE_3 != 0 {
            next |= STATE_COUNT;
        }
        if state & STATE_FORCE_LOAD_CONTROL != 0 {
            next |= STATE_LOAD_STAGE_1;
        }
        if state & STATE_LOAD_STAGE_1 != 0 {
            next |= STATE_LOAD;
        }
        if state & STATE_ONE_SHOT_CONTROL != 0 {
            next |= STATE_ONE_SHOT_STAGE_0;
        }
        if state & STATE_ONE_SHOT_STAGE_0 != 0 {
            next |= STATE_ONE_SHOT;
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::{Mos6526Timer, TimerInputMode};
    use crate::devices::cia::{CONTROL_FORCE_LOAD, CONTROL_ONE_SHOT, CONTROL_START};

    #[test]
    fn force_load_and_count_are_separate_pipeline_events() {
        let mut timer = Mos6526Timer::new();
        timer.write_latch_low(1);
        timer.write_latch_high(0);
        timer.write_control(
            CONTROL_START | CONTROL_FORCE_LOAD,
            TimerInputMode::ProcessorClock,
        );
        assert!(!timer.tick_cycle(false));
        assert!(!timer.tick_cycle(false));
        assert!(!timer.tick_cycle(false));
        assert_eq!(timer.counter(), 1);
        assert!(timer.tick_cycle(false));
        assert_eq!(timer.counter(), 1);
        assert!(timer.output_high());
        assert!(!timer.tick_cycle(false));
        assert!(!timer.output_high());
    }

    #[test]
    fn one_shot_clears_running_after_first_underflow() {
        let mut timer = Mos6526Timer::new();
        timer.write_latch_low(1);
        timer.write_latch_high(0);
        timer.write_control(
            CONTROL_START | CONTROL_FORCE_LOAD | CONTROL_ONE_SHOT,
            TimerInputMode::ProcessorClock,
        );
        for _ in 0..4 {
            timer.tick_cycle(false);
        }
        assert!(!timer.running());
        assert_eq!(timer.counter(), 1);
    }
}
