// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - SID oscillator/envelope voice
//
//   File:       sid/voice.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use super::{SidEnvelopeGenerator, SidModel, SidOscillator};

const MOS6581_WAVEFORM_DAC_ZERO: i32 = 0x0380;
const MOS8580_WAVEFORM_DAC_ZERO: i32 = 0x09e0;

/// One SID voice, combining the digital oscillator, ADSR generator and
/// model-specific waveform DAC zero level.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidVoice {
    attack_decay_register: u8,
    sustain_release_register: u8,
    envelope: SidEnvelopeGenerator,
    oscillator: SidOscillator,
    waveform_dac_zero: i32,
}

impl SidVoice {
    pub fn new(model: SidModel) -> Self {
        Self {
            attack_decay_register: 0,
            sustain_release_register: 0,
            envelope: SidEnvelopeGenerator::new(),
            oscillator: SidOscillator::new(model),
            waveform_dac_zero: match model {
                SidModel::Mos6581 => MOS6581_WAVEFORM_DAC_ZERO,
                SidModel::Mos8580 => MOS8580_WAVEFORM_DAC_ZERO,
            },
        }
    }

    pub const fn model(&self) -> SidModel {
        self.oscillator.model()
    }

    pub const fn frequency(&self) -> u16 {
        self.oscillator.frequency()
    }

    pub fn set_frequency(&mut self, value: u16) {
        self.oscillator.set_frequency(value);
    }

    pub const fn pulse_width(&self) -> u16 {
        self.oscillator.pulse_width()
    }

    pub fn set_pulse_width(&mut self, value: u16) {
        self.oscillator.set_pulse_width(value);
    }

    pub const fn control(&self) -> u8 {
        self.oscillator.control()
    }

    pub const fn accumulator(&self) -> u32 {
        self.oscillator.accumulator()
    }

    pub const fn accumulator_most_significant_bit(&self) -> bool {
        self.oscillator.accumulator_most_significant_bit()
    }

    pub const fn oscillator_msb_rising(&self) -> bool {
        self.oscillator.msb_rising()
    }

    pub const fn oscillator_sync_enabled(&self) -> bool {
        self.oscillator.sync_enabled()
    }

    pub const fn envelope_output(&self) -> u8 {
        self.envelope.output()
    }

    pub const fn envelope_readback(&self) -> u8 {
        self.envelope.readback()
    }

    /// Multiplying DAC level presented to the model-specific analog filter.
    pub fn analog_output(&self) -> i32 {
        (i32::from(self.oscillator.waveform_output()) - self.waveform_dac_zero)
            * i32::from(self.envelope.output())
    }

    pub const fn attack_decay(&self) -> u8 {
        self.attack_decay_register
    }

    pub fn set_attack_decay(&mut self, value: u8) {
        self.attack_decay_register = value;
        self.envelope.write_attack_decay(value);
    }

    pub const fn sustain_release(&self) -> u8 {
        self.sustain_release_register
    }

    pub fn set_sustain_release(&mut self, value: u8) {
        self.sustain_release_register = value;
        self.envelope.write_sustain_release(value);
    }

    pub fn reset(&mut self) {
        self.attack_decay_register = 0;
        self.sustain_release_register = 0;
        self.oscillator.reset();
        self.envelope.reset();
    }

    pub fn set_control_with_sync_source(&mut self, value: u8, source_accumulator: u32) {
        self.oscillator
            .set_control_with_sync_source(value, source_accumulator);
        self.envelope.write_control(value);
    }

    pub fn clock_oscillator(&mut self) {
        self.oscillator.clock_cycle();
    }

    pub fn reset_accumulator_for_sync(&mut self) {
        self.oscillator.reset_accumulator_for_sync();
    }

    pub fn update_waveform_output_with_sync_source(&mut self, source_accumulator: u32) {
        self.oscillator
            .update_waveform_output_with_sync_source(source_accumulator);
    }

    pub fn clock_envelope(&mut self) {
        self.envelope.clock_cycle();
    }

    pub const fn waveform_output(&self) -> u16 {
        self.oscillator.waveform_output()
    }

    pub const fn oscillator_readback(&self) -> u8 {
        self.oscillator.oscillator_readback()
    }
}

#[cfg(test)]
mod tests {
    use super::SidVoice;
    use crate::devices::sid::{SidModel, control};

    #[test]
    fn model_specific_waveform_dac_zero_changes_analog_level() {
        let mut mos6581 = SidVoice::new(SidModel::Mos6581);
        let mut mos8580 = SidVoice::new(SidModel::Mos8580);
        mos6581.set_control_with_sync_source(control::TEST | control::PULSE, 0);
        mos8580.set_control_with_sync_source(control::TEST | control::PULSE, 0);
        mos6581.set_attack_decay(0);
        mos8580.set_attack_decay(0);
        mos6581.set_control_with_sync_source(control::TEST | control::PULSE | control::GATE, 0);
        mos8580.set_control_with_sync_source(control::TEST | control::PULSE | control::GATE, 0);
        for _ in 0..32 {
            mos6581.clock_envelope();
            mos8580.clock_envelope();
        }

        assert_ne!(mos6581.analog_output(), mos8580.analog_output());
    }
}
