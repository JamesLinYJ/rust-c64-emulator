// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - SID 24-bit oscillator and waveform pipeline
//
//   File:       sid/oscillator.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use super::waveform_data::{
    SID_6581_PULSE_SAW, SID_6581_PULSE_SAW_TRIANGLE, SID_6581_PULSE_TRIANGLE,
    SID_6581_TRIANGLE_SAW, SID_8580_PULSE_SAW, SID_8580_PULSE_SAW_TRIANGLE,
    SID_8580_PULSE_TRIANGLE, SID_8580_TRIANGLE_SAW,
};
use super::{SidModel, control};

const OSCILLATOR_POWER_ON_ACCUMULATOR: u32 = 0x55_5555;
const NOISE_SHIFT_REGISTER_MASK: u32 = 0x7f_ffff;
const NOISE_SHIFT_REGISTER_RESET: u32 = 0x7f_fffe;
const NOISE_SHIFT_TRIGGER_BIT: u32 = 0x08_0000;
const OSCILLATOR_MSB: u32 = 0x80_0000;
const ACCUMULATOR_MASK: u32 = 0xff_ffff;
const PHASE_WAVEFORM_MASK: u16 = 0x0fff;
const PULSE_WIDTH_MASK: u16 = 0x0fff;
const WAVEFORM_SELECTION_MASK: u8 = 0x0f;
const WAVEFORM_TABLE_INDEX_MASK: u8 = 0x07;
const WAVEFORM_NOISE_BIT: u8 = 0x08;
const WAVEFORM_NOISE_TRIANGLE: u8 = 0x09;
const WAVEFORM_PULSE_BIT: u8 = 0x04;
const WAVEFORM_SAWTOOTH_BIT: u8 = 0x02;
const WAVEFORM_TRIANGLE_SAW_MASK: u8 = 0x03;
const WAVEFORM_NOISE_PULSE_MASK: u8 = 0x0c;
const COMBINED_WAVEFORM_FIRST_INDEX: u8 = 0x09;
const NOISE_OUTPUT_REGISTER_MASK: u32 =
    (1 << 20) | (1 << 18) | (1 << 14) | (1 << 11) | (1 << 9) | (1 << 5) | (1 << 2) | 1;
const STATE_TEST_ENABLED: u8 = 1 << 0;
const STATE_RING_MODULATION_ENABLED: u8 = 1 << 1;
const STATE_SYNC_ENABLED: u8 = 1 << 2;
const STATE_MSB_RISING: u8 = 1 << 3;

#[derive(Clone, Copy)]
struct SidOscillatorTiming {
    floating_output_first: u32,
    floating_output_next: u32,
    noise_reset_first: u32,
    noise_reset_next: u32,
}

const MOS6581_TIMING: SidOscillatorTiming = SidOscillatorTiming {
    floating_output_first: 182_000,
    floating_output_next: 1_500,
    noise_reset_first: 35_000,
    noise_reset_next: 1_000,
};

const MOS8580_TIMING: SidOscillatorTiming = SidOscillatorTiming {
    floating_output_first: 4_400_000,
    floating_output_next: 50_000,
    noise_reset_first: 2_519_864,
    noise_reset_next: 315_000,
};

/// 单声部 24 位相位累加器、噪声 LFSR 以及 MOS 6581/8580 数字波形通路。
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct SidOscillator {
    accumulator: u32,
    frequency_register: u16,
    pulse_width_register: u16,
    control_register: u8,
    waveform_selection: u8,
    model: SidModel,
    state_flags: u8,
    noise_shift_register: u32,
    noise_shift_reset_cycles: u32,
    noise_shift_pipeline: u8,
    noise_output: u16,
    no_noise_mask: u16,
    no_noise_or_noise_output: u16,
    no_pulse_mask: u16,
    pulse_output: u16,
    ring_most_significant_bit_mask: u32,
    waveform_output: u16,
    oscillator_readback: u16,
    triangle_saw_pipeline: u16,
    floating_output_cycles: u32,
}

impl Default for SidOscillator {
    fn default() -> Self {
        Self::new(SidModel::Mos6581)
    }
}

impl SidOscillator {
    pub fn new(model: SidModel) -> Self {
        let mut result = Self {
            accumulator: OSCILLATOR_POWER_ON_ACCUMULATOR,
            frequency_register: 0,
            pulse_width_register: 0,
            control_register: 0,
            waveform_selection: 0,
            model,
            state_flags: 0,
            noise_shift_register: NOISE_SHIFT_REGISTER_RESET,
            noise_shift_reset_cycles: 0,
            noise_shift_pipeline: 0,
            noise_output: 0,
            no_noise_mask: PHASE_WAVEFORM_MASK,
            no_noise_or_noise_output: PHASE_WAVEFORM_MASK,
            no_pulse_mask: PHASE_WAVEFORM_MASK,
            pulse_output: PHASE_WAVEFORM_MASK,
            ring_most_significant_bit_mask: 0,
            waveform_output: 0,
            oscillator_readback: 0,
            triangle_saw_pipeline: 0x0555,
            floating_output_cycles: 0,
        };
        result.reset();
        result
    }

    pub const fn model(&self) -> SidModel {
        self.model
    }

    pub const fn accumulator(&self) -> u32 {
        self.accumulator
    }

    pub const fn accumulator_most_significant_bit(&self) -> bool {
        self.accumulator & OSCILLATOR_MSB != 0
    }

    pub const fn msb_rising(&self) -> bool {
        self.state_flag(STATE_MSB_RISING)
    }

    pub const fn sync_enabled(&self) -> bool {
        self.state_flag(STATE_SYNC_ENABLED)
    }

    pub const fn frequency(&self) -> u16 {
        self.frequency_register
    }

    pub fn set_frequency(&mut self, frequency: u16) {
        self.frequency_register = frequency;
    }

    pub const fn pulse_width(&self) -> u16 {
        self.pulse_width_register
    }

    pub fn set_pulse_width(&mut self, pulse_width: u16) {
        self.pulse_width_register = pulse_width & PULSE_WIDTH_MASK;
        self.pulse_output = if (self.accumulator >> 12) >= u32::from(self.pulse_width_register) {
            PHASE_WAVEFORM_MASK
        } else {
            0
        };
    }

    pub const fn control(&self) -> u8 {
        self.control_register
    }

    pub const fn waveform_output(&self) -> u16 {
        self.waveform_output
    }

    pub const fn oscillator_readback(&self) -> u8 {
        (self.oscillator_readback >> 4).to_le_bytes()[0]
    }

    /// `/RES` 清除寄存器与数字控制管线，但不连接相位累加器或 8580 tri/saw 管线。
    pub fn reset(&mut self) {
        self.frequency_register = 0;
        self.pulse_width_register = 0;
        self.control_register = 0;
        self.waveform_selection = 0;
        self.state_flags = 0;
        self.noise_shift_register = NOISE_SHIFT_REGISTER_RESET;
        self.noise_shift_reset_cycles = 0;
        self.noise_shift_pipeline = 0;
        self.update_noise_output();
        self.no_noise_mask = PHASE_WAVEFORM_MASK;
        self.no_noise_or_noise_output = PHASE_WAVEFORM_MASK;
        self.no_pulse_mask = PHASE_WAVEFORM_MASK;
        self.pulse_output = PHASE_WAVEFORM_MASK;
        self.ring_most_significant_bit_mask = 0;
        self.waveform_output = 0;
        self.oscillator_readback = 0;
        self.floating_output_cycles = 0;
    }

    pub fn set_control(&mut self, value: u8) {
        self.set_control_with_sync_source(value, self.accumulator);
    }

    pub fn set_control_with_sync_source(&mut self, value: u8, sync_source_accumulator: u32) {
        let previous_waveform = self.waveform_selection;
        let previous_test = self.state_flag(STATE_TEST_ENABLED);
        self.control_register = value;
        self.waveform_selection = (value >> 4) & WAVEFORM_SELECTION_MASK;
        self.set_state_flag(STATE_TEST_ENABLED, value & control::TEST != 0);
        self.set_state_flag(
            STATE_RING_MODULATION_ENABLED,
            value & control::RING_MODULATION != 0,
        );
        self.set_state_flag(STATE_SYNC_ENABLED, value & control::SYNCHRONIZE != 0);
        self.ring_most_significant_bit_mask =
            if self.state_flag(STATE_RING_MODULATION_ENABLED) && value & control::SAWTOOTH == 0 {
                OSCILLATOR_MSB
            } else {
                0
            };
        self.no_noise_mask = if self.waveform_selection & WAVEFORM_NOISE_BIT != 0 {
            0
        } else {
            PHASE_WAVEFORM_MASK
        };
        self.no_noise_or_noise_output = self.no_noise_mask | self.noise_output;
        self.no_pulse_mask = if self.waveform_selection & WAVEFORM_PULSE_BIT != 0 {
            0
        } else {
            PHASE_WAVEFORM_MASK
        };

        if !previous_test && self.state_flag(STATE_TEST_ENABLED) {
            self.accumulator = 0;
            self.noise_shift_pipeline = 0;
            self.noise_shift_reset_cycles = self.timing().noise_reset_first;
            self.pulse_output = PHASE_WAVEFORM_MASK;
        } else if previous_test && !self.state_flag(STATE_TEST_ENABLED) {
            if self
                .should_write_back_before_test_release(previous_waveform, self.waveform_selection)
            {
                self.write_noise_shift_register();
            }
            let feedback = (!self.noise_shift_register >> 17) & 1;
            self.noise_shift_register =
                ((self.noise_shift_register << 1) | feedback) & NOISE_SHIFT_REGISTER_MASK;
            self.update_noise_output();
        }

        if self.waveform_selection == 0 {
            if previous_waveform != 0 {
                self.floating_output_cycles = self.timing().floating_output_first;
            }
        } else {
            self.update_waveform_output_with_sync_source(sync_source_accumulator);
        }
    }

    pub fn clock_cycle(&mut self) {
        if self.state_flag(STATE_TEST_ENABLED) {
            if self.noise_shift_reset_cycles != 0 {
                self.noise_shift_reset_cycles -= 1;
                if self.noise_shift_reset_cycles == 0 {
                    self.fade_noise_shift_register();
                }
            }
            self.pulse_output = PHASE_WAVEFORM_MASK;
            return;
        }

        let previous_accumulator = self.accumulator;
        let next_accumulator = previous_accumulator
            .wrapping_add(u32::from(self.frequency_register))
            & ACCUMULATOR_MASK;
        let accumulator_bits_set = !previous_accumulator & next_accumulator;
        self.accumulator = next_accumulator;
        self.set_state_flag(STATE_MSB_RISING, accumulator_bits_set & OSCILLATOR_MSB != 0);

        if accumulator_bits_set & NOISE_SHIFT_TRIGGER_BIT != 0 {
            self.noise_shift_pipeline = 2;
        } else if self.noise_shift_pipeline != 0 {
            self.noise_shift_pipeline -= 1;
            if self.noise_shift_pipeline == 0 {
                self.clock_noise_shift_register();
            }
        }
    }

    pub fn reset_accumulator_for_sync(&mut self) {
        self.accumulator = 0;
    }

    pub fn update_waveform_output(&mut self) {
        self.update_waveform_output_with_sync_source(self.accumulator);
    }

    pub fn update_waveform_output_with_sync_source(&mut self, sync_source_accumulator: u32) {
        if self.waveform_selection != 0 {
            let phase_index = low_u16(
                (self.accumulator
                    ^ (!sync_source_accumulator & self.ring_most_significant_bit_mask))
                    >> 12,
            ) & PHASE_WAVEFORM_MASK;
            let table_value = waveform_value(
                self.model,
                self.waveform_selection & WAVEFORM_TABLE_INDEX_MASK,
                phase_index,
            );
            self.waveform_output = table_value
                & (self.no_pulse_mask | self.pulse_output)
                & self.no_noise_or_noise_output;

            if self.waveform_selection & WAVEFORM_NOISE_PULSE_MASK == WAVEFORM_NOISE_PULSE_MASK {
                self.waveform_output = self.apply_noise_pulse_coupling(self.waveform_output);
            }

            if self.waveform_selection & WAVEFORM_TRIANGLE_SAW_MASK != 0
                && self.model == SidModel::Mos8580
            {
                self.oscillator_readback = self.triangle_saw_pipeline
                    & (self.no_pulse_mask | self.pulse_output)
                    & self.no_noise_or_noise_output;
                self.triangle_saw_pipeline = table_value;
            } else {
                self.oscillator_readback = self.waveform_output;
            }

            if self.waveform_selection & WAVEFORM_SAWTOOTH_BIT != 0
                && self.waveform_selection & 0x0d != 0
                && self.model == SidModel::Mos6581
            {
                self.accumulator &=
                    (u32::from(self.waveform_output) << 12) | (ACCUMULATOR_MASK ^ OSCILLATOR_MSB);
            }

            if self.waveform_selection >= COMBINED_WAVEFORM_FIRST_INDEX
                && !self.state_flag(STATE_TEST_ENABLED)
                && self.noise_shift_pipeline != 1
            {
                self.write_noise_shift_register();
            }
        } else if self.floating_output_cycles != 0 {
            self.floating_output_cycles -= 1;
            if self.floating_output_cycles == 0 {
                self.fade_floating_waveform_output();
            }
        }

        self.pulse_output = if (self.accumulator >> 12) >= u32::from(self.pulse_width_register) {
            PHASE_WAVEFORM_MASK
        } else {
            0
        };
    }

    const fn timing(&self) -> SidOscillatorTiming {
        match self.model {
            SidModel::Mos6581 => MOS6581_TIMING,
            SidModel::Mos8580 => MOS8580_TIMING,
        }
    }

    fn clock_noise_shift_register(&mut self) {
        let feedback = ((self.noise_shift_register >> 22) ^ (self.noise_shift_register >> 17)) & 1;
        self.noise_shift_register =
            ((self.noise_shift_register << 1) | feedback) & NOISE_SHIFT_REGISTER_MASK;
        self.update_noise_output();
    }

    fn update_noise_output(&mut self) {
        self.noise_output = low_u16(
            ((self.noise_shift_register & 0x10_0000) >> 9)
                | ((self.noise_shift_register & 0x04_0000) >> 8)
                | ((self.noise_shift_register & 0x00_4000) >> 5)
                | ((self.noise_shift_register & 0x00_0800) >> 3)
                | ((self.noise_shift_register & 0x00_0200) >> 2)
                | ((self.noise_shift_register & 0x00_0020) << 1)
                | ((self.noise_shift_register & 0x00_0004) << 3)
                | ((self.noise_shift_register & 0x00_0001) << 4),
        );
        self.no_noise_or_noise_output = self.no_noise_mask | self.noise_output;
    }

    fn write_noise_shift_register(&mut self) {
        self.noise_shift_register &= !NOISE_OUTPUT_REGISTER_MASK
            | ((u32::from(self.waveform_output) & 0x0800) << 9)
            | ((u32::from(self.waveform_output) & 0x0400) << 8)
            | ((u32::from(self.waveform_output) & 0x0200) << 5)
            | ((u32::from(self.waveform_output) & 0x0100) << 3)
            | ((u32::from(self.waveform_output) & 0x0080) << 2)
            | ((u32::from(self.waveform_output) & 0x0040) >> 1)
            | ((u32::from(self.waveform_output) & 0x0020) >> 3)
            | ((u32::from(self.waveform_output) & 0x0010) >> 4);
        self.noise_shift_register &= NOISE_SHIFT_REGISTER_MASK;
        self.noise_output &= self.waveform_output;
        self.no_noise_or_noise_output = self.no_noise_mask | self.noise_output;
    }

    fn fade_floating_waveform_output(&mut self) {
        self.waveform_output &= self.waveform_output >> 1;
        self.oscillator_readback = self.waveform_output;
        if self.waveform_output != 0 {
            self.floating_output_cycles = self.timing().floating_output_next;
        }
    }

    fn fade_noise_shift_register(&mut self) {
        self.noise_shift_register |= 1;
        self.noise_shift_register |= self.noise_shift_register << 1;
        self.noise_shift_register &= NOISE_SHIFT_REGISTER_MASK;
        self.update_noise_output();
        if self.noise_shift_register != NOISE_SHIFT_REGISTER_MASK {
            self.noise_shift_reset_cycles = self.timing().noise_reset_next;
        }
    }

    const fn apply_noise_pulse_coupling(&self, noise: u16) -> u16 {
        match self.model {
            SidModel::Mos6581 => {
                if noise < 0x0f00 {
                    0
                } else {
                    noise & (noise << 1) & (noise << 2)
                }
            }
            SidModel::Mos8580 => {
                if noise < 0x0fc0 {
                    noise & (noise << 1)
                } else {
                    0x0fc0
                }
            }
        }
    }

    fn should_write_back_before_test_release(
        &self,
        previous_waveform: u8,
        next_waveform: u8,
    ) -> bool {
        if previous_waveform <= WAVEFORM_NOISE_BIT
            || (previous_waveform == WAVEFORM_NOISE_TRIANGLE && next_waveform == WAVEFORM_NOISE_BIT)
        {
            return false;
        }
        if previous_waveform == WAVEFORM_NOISE_PULSE_MASK
            && (self.model == SidModel::Mos6581 || (next_waveform != 0x09 && next_waveform != 0x0e))
        {
            return false;
        }
        if self.model == SidModel::Mos6581
            && (((previous_waveform & WAVEFORM_TRIANGLE_SAW_MASK) == 1
                && (next_waveform & WAVEFORM_TRIANGLE_SAW_MASK) == 2)
                || ((previous_waveform & WAVEFORM_TRIANGLE_SAW_MASK) == 2
                    && (next_waveform & WAVEFORM_TRIANGLE_SAW_MASK) == 1))
        {
            return false;
        }
        true
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

fn waveform_value(model: SidModel, waveform_index: u8, phase: u16) -> u16 {
    let phase_index = usize::from(phase);
    match (model, waveform_index) {
        (_, 0 | 4) => PHASE_WAVEFORM_MASK,
        (_, 1) => {
            let accumulator = u32::from(phase) << 12;
            let inverted_phase_mask = if accumulator & OSCILLATOR_MSB != 0 {
                u32::MAX
            } else {
                0
            };
            low_u16((accumulator ^ inverted_phase_mask) >> 11) & 0x0ffe
        }
        (_, 2) => phase,
        (SidModel::Mos6581, 3) => u16::from(SID_6581_TRIANGLE_SAW[phase_index]) << 4,
        (SidModel::Mos6581, 5) => u16::from(SID_6581_PULSE_TRIANGLE[phase_index]) << 4,
        (SidModel::Mos6581, 6) => u16::from(SID_6581_PULSE_SAW[phase_index]) << 4,
        (SidModel::Mos8580, 3) => u16::from(SID_8580_TRIANGLE_SAW[phase_index]) << 4,
        (SidModel::Mos8580, 5) => u16::from(SID_8580_PULSE_TRIANGLE[phase_index]) << 4,
        (SidModel::Mos8580, 6) => u16::from(SID_8580_PULSE_SAW[phase_index]) << 4,
        (SidModel::Mos8580, _) => u16::from(SID_8580_PULSE_SAW_TRIANGLE[phase_index]) << 4,
        (SidModel::Mos6581, _) => u16::from(SID_6581_PULSE_SAW_TRIANGLE[phase_index]) << 4,
    }
}

const fn low_u16(value: u32) -> u16 {
    let bytes = value.to_le_bytes();
    u16::from_le_bytes([bytes[0], bytes[1]])
}

#[cfg(test)]
mod tests {
    use super::SidOscillator;
    use crate::devices::sid::{SidModel, control};

    #[test]
    fn mos8580_triangle_saw_readback_is_delayed_by_one_cycle() {
        let mut mos6581 = SidOscillator::new(SidModel::Mos6581);
        let mut mos8580 = SidOscillator::new(SidModel::Mos8580);
        for oscillator in [&mut mos6581, &mut mos8580] {
            oscillator.set_frequency(u16::MAX);
            oscillator.set_control(control::SAWTOOTH);
            oscillator.clock_cycle();
            oscillator.update_waveform_output();
        }

        let current_6581 = mos6581.oscillator_readback();
        assert_ne!(mos8580.oscillator_readback(), current_6581);
        mos8580.clock_cycle();
        mos8580.update_waveform_output();
        assert_eq!(mos8580.oscillator_readback(), current_6581);
    }

    #[test]
    fn reset_preserves_phase_while_clearing_waveform_selection() {
        let mut reset = SidOscillator::new(SidModel::Mos6581);
        let mut uninterrupted = SidOscillator::new(SidModel::Mos6581);
        for oscillator in [&mut reset, &mut uninterrupted] {
            oscillator.set_frequency(0x4321);
            oscillator.set_control(control::SAWTOOTH);
            for _ in 0..40 {
                oscillator.clock_cycle();
            }
        }

        reset.reset();
        reset.set_control(control::SAWTOOTH);
        uninterrupted.update_waveform_output();
        assert_eq!(
            reset.oscillator_readback(),
            uninterrupted.oscillator_readback()
        );
    }

    #[test]
    fn test_noise_triangle_to_noise_does_not_prewrite_the_lfsr() {
        let mut oscillator = SidOscillator::new(SidModel::Mos6581);
        oscillator.set_frequency(0);
        oscillator.set_control(control::TEST | control::NOISE | control::TRIANGLE);
        for _ in 0..60_000 {
            oscillator.clock_cycle();
            oscillator.update_waveform_output();
        }

        oscillator.set_control(control::NOISE);
        assert_eq!(oscillator.oscillator_readback(), 0xfe);
        oscillator.set_frequency(u16::MAX);
        for _ in 0..11 {
            oscillator.clock_cycle();
            oscillator.update_waveform_output();
        }
        assert_eq!(oscillator.oscillator_readback(), 0xfe);
    }
}
