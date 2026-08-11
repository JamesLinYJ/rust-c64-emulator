// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - SID ADSR envelope generator
//
//   File:       sid/envelope.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use super::{ENVELOPE_RATE_COMPARE_VALUES, control};

const ENVELOPE_COUNTER_MAXIMUM: u8 = u8::MAX;
const RATE_COUNTER_OVERFLOW_BIT: u16 = 0x8000;
const RATE_COUNTER_MASK: u16 = 0x7fff;
const POWER_ON_ENVELOPE_COUNTER: u8 = 0xaa;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnvelopeState {
    Attack,
    DecaySustain,
    Release,
}

/// SID 数字 ADSR 单元，包含硬件流水线延迟与 15 位计数器绕回行为。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidEnvelopeGenerator {
    attack: u8,
    decay: u8,
    sustain: u8,
    release: u8,
    gate: bool,
    state: EnvelopeState,
    next_state: EnvelopeState,
    state_pipeline: u8,
    envelope_counter: u8,
    envelope_readback: u8,
    envelope_pipeline: u8,
    exponential_counter: u8,
    exponential_counter_period: u8,
    exponential_pipeline: u8,
    hold_zero: bool,
    rate_counter: u16,
    rate_period: u16,
    reset_rate_counter: bool,
}

impl Default for SidEnvelopeGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl SidEnvelopeGenerator {
    pub const fn new() -> Self {
        Self {
            attack: 0,
            decay: 0,
            sustain: 0,
            release: 0,
            gate: false,
            state: EnvelopeState::Release,
            next_state: EnvelopeState::Release,
            state_pipeline: 0,
            envelope_counter: POWER_ON_ENVELOPE_COUNTER,
            envelope_readback: POWER_ON_ENVELOPE_COUNTER,
            envelope_pipeline: 0,
            exponential_counter: 0,
            exponential_counter_period: 1,
            exponential_pipeline: 0,
            hold_zero: false,
            rate_counter: 0,
            rate_period: ENVELOPE_RATE_COMPARE_VALUES[0],
            reset_rate_counter: false,
        }
    }

    pub const fn output(&self) -> u8 {
        self.envelope_counter
    }

    pub const fn readback(&self) -> u8 {
        self.envelope_readback
    }

    /// `/RES` 不连接八位包络计数器，因此保留当前模拟电平，只复位控制状态。
    pub fn reset(&mut self) {
        self.attack = 0;
        self.decay = 0;
        self.sustain = 0;
        self.release = 0;
        self.gate = false;
        self.state = EnvelopeState::Release;
        self.state_pipeline = 0;
        self.envelope_pipeline = 0;
        self.exponential_counter = 0;
        self.exponential_counter_period = 1;
        self.exponential_pipeline = 0;
        self.hold_zero = false;
        self.rate_counter = 0;
        self.rate_period = Self::rate_value(self.release);
        self.reset_rate_counter = false;
        self.envelope_readback = self.envelope_counter;
    }

    pub fn write_control(&mut self, value: u8) {
        let next_gate = value & control::GATE != 0;
        if self.gate == next_gate {
            return;
        }

        self.next_state = if next_gate {
            EnvelopeState::Attack
        } else {
            EnvelopeState::Release
        };
        if self.next_state == EnvelopeState::Attack {
            self.state = EnvelopeState::DecaySustain;
            self.rate_period = Self::rate_value(self.decay);
            self.state_pipeline = 2;
            if self.reset_rate_counter || self.exponential_pipeline == 2 {
                self.envelope_pipeline =
                    if self.exponential_counter_period == 1 || self.exponential_pipeline == 2 {
                        2
                    } else {
                        4
                    };
            } else if self.exponential_pipeline == 1 {
                self.state_pipeline = 3;
            }
        } else {
            self.state_pipeline = if self.envelope_pipeline > 0 { 3 } else { 2 };
        }
        self.gate = next_gate;
    }

    pub fn write_attack_decay(&mut self, value: u8) {
        self.attack = value >> 4;
        self.decay = value & 0x0f;
        if self.state == EnvelopeState::Attack {
            self.rate_period = Self::rate_value(self.attack);
        } else if self.state == EnvelopeState::DecaySustain {
            self.rate_period = Self::rate_value(self.decay);
        }
    }

    pub fn write_sustain_release(&mut self, value: u8) {
        self.sustain = value >> 4;
        self.release = value & 0x0f;
        if self.state == EnvelopeState::Release {
            self.rate_period = Self::rate_value(self.release);
        }
    }

    pub fn clock_cycle(&mut self) {
        self.envelope_readback = self.envelope_counter;

        if self.state_pipeline != 0 {
            self.advance_state_pipeline();
        }

        if self.envelope_pipeline != 0 {
            self.envelope_pipeline -= 1;
            if self.envelope_pipeline == 0 && !self.hold_zero {
                if self.state == EnvelopeState::Attack {
                    self.envelope_counter = self.envelope_counter.wrapping_add(1);
                    if self.envelope_counter == ENVELOPE_COUNTER_MAXIMUM {
                        self.state = EnvelopeState::DecaySustain;
                        self.rate_period = Self::rate_value(self.decay);
                    }
                } else {
                    self.envelope_counter = self.envelope_counter.wrapping_sub(1);
                }
                self.update_exponential_period();
            }
        }

        if self.exponential_pipeline != 0 {
            self.exponential_pipeline -= 1;
            if self.exponential_pipeline == 0 {
                self.exponential_counter = 0;
                if (self.state == EnvelopeState::DecaySustain
                    && self.envelope_counter != self.sustain_level())
                    || self.state == EnvelopeState::Release
                {
                    self.envelope_pipeline = 1;
                }
            }
        } else if self.reset_rate_counter {
            self.rate_counter = 0;
            self.reset_rate_counter = false;

            if self.state == EnvelopeState::Attack {
                self.exponential_counter = 0;
                self.envelope_pipeline = 2;
            } else if !self.hold_zero {
                self.exponential_counter += 1;
                if self.exponential_counter == self.exponential_counter_period {
                    self.exponential_pipeline = if self.exponential_counter_period == 1 {
                        1
                    } else {
                        2
                    };
                }
            }
        }

        if self.rate_counter == self.rate_period {
            self.reset_rate_counter = true;
        } else {
            self.rate_counter += 1;
            if self.rate_counter & RATE_COUNTER_OVERFLOW_BIT != 0 {
                self.rate_counter = self.rate_counter.wrapping_add(1) & RATE_COUNTER_MASK;
            }
        }
    }

    fn advance_state_pipeline(&mut self) {
        self.state_pipeline -= 1;
        if self.next_state == EnvelopeState::Attack && self.state_pipeline == 0 {
            self.state = EnvelopeState::Attack;
            self.rate_period = Self::rate_value(self.attack);
            self.hold_zero = false;
            return;
        }
        if self.next_state == EnvelopeState::Release
            && ((self.state == EnvelopeState::Attack && self.state_pipeline == 0)
                || (self.state == EnvelopeState::DecaySustain && self.state_pipeline == 1))
        {
            self.state = EnvelopeState::Release;
            self.rate_period = Self::rate_value(self.release);
        }
    }

    fn update_exponential_period(&mut self) {
        self.exponential_counter_period = match self.envelope_counter {
            0xff | 0x00 => 1,
            0x5d => 2,
            0x36 => 4,
            0x1a => 8,
            0x0e => 16,
            0x06 => 30,
            _ => self.exponential_counter_period,
        };
        if self.envelope_counter == 0 {
            self.hold_zero = true;
        }
    }

    const fn sustain_level(&self) -> u8 {
        self.sustain * 0x11
    }

    const fn rate_value(index: u8) -> u16 {
        ENVELOPE_RATE_COMPARE_VALUES[index as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::SidEnvelopeGenerator;
    use crate::devices::sid::control;

    #[test]
    fn reset_preserves_counter_but_resets_control_pipeline() {
        let mut envelope = SidEnvelopeGenerator::new();
        envelope.write_attack_decay(0x00);
        envelope.write_control(control::GATE);
        for _ in 0..100 {
            envelope.clock_cycle();
        }
        let output_before_reset = envelope.output();

        envelope.reset();

        assert_eq!(envelope.output(), output_before_reset);
        assert_eq!(envelope.readback(), output_before_reset);
    }

    #[test]
    fn env3_readback_precedes_the_current_envelope_level() {
        let mut envelope = SidEnvelopeGenerator::new();
        envelope.write_attack_decay(0x00);
        envelope.write_control(control::GATE);

        let mut observed_pipeline_delay = false;
        for _ in 0..100 {
            envelope.clock_cycle();
            if envelope.output() != envelope.readback() {
                observed_pipeline_delay = true;
                break;
            }
        }
        assert!(observed_pipeline_delay);
    }
}
