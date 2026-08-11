// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - MOS 6581 nonlinear filter state machine
//
//   File:       sid/filter_6581.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use super::filter_6581_model::{
    Sid6581FilterModel, Sid6581IntegratorState, shared_mos6581_filter_model,
};
use super::filter_bit;

/// Per-chip capacitor and register state for the nonlinear MOS 6581 filter.
#[derive(Clone)]
pub struct SidMos6581Filter {
    model: &'static Sid6581FilterModel,
    band_pass_integrator: Sid6581IntegratorState,
    low_pass_integrator: Sid6581IntegratorState,
    band_pass_voltage: i32,
    cutoff_register: u16,
    cutoff_voltage_squared: i64,
    high_pass_voltage: i32,
    low_pass_voltage: i32,
    resonance_routing: u8,
    mode_volume: u8,
    output_pcm_state: i16,
}

impl fmt::Debug for SidMos6581Filter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SidMos6581Filter")
            .field("model", &self.model)
            .field("band_pass_integrator", &self.band_pass_integrator)
            .field("low_pass_integrator", &self.low_pass_integrator)
            .field("band_pass_voltage", &self.band_pass_voltage)
            .field("cutoff_register", &self.cutoff_register)
            .field("cutoff_voltage_squared", &self.cutoff_voltage_squared)
            .field("high_pass_voltage", &self.high_pass_voltage)
            .field("low_pass_voltage", &self.low_pass_voltage)
            .field("resonance_routing", &self.resonance_routing)
            .field("mode_volume", &self.mode_volume)
            .field("output_pcm_state", &self.output_pcm_state)
            .finish()
    }
}

impl PartialEq for SidMos6581Filter {
    fn eq(&self, other: &Self) -> bool {
        self.band_pass_integrator == other.band_pass_integrator
            && self.low_pass_integrator == other.low_pass_integrator
            && self.band_pass_voltage == other.band_pass_voltage
            && self.cutoff_register == other.cutoff_register
            && self.cutoff_voltage_squared == other.cutoff_voltage_squared
            && self.high_pass_voltage == other.high_pass_voltage
            && self.low_pass_voltage == other.low_pass_voltage
            && self.resonance_routing == other.resonance_routing
            && self.mode_volume == other.mode_volume
            && self.output_pcm_state == other.output_pcm_state
    }
}

impl Eq for SidMos6581Filter {}

impl Default for SidMos6581Filter {
    fn default() -> Self {
        Self::new()
    }
}

impl SidMos6581Filter {
    pub fn new() -> Self {
        let model = shared_mos6581_filter_model();
        let mut result = Self {
            model,
            band_pass_integrator: Sid6581FilterModel::create_integrator_state(),
            low_pass_integrator: Sid6581FilterModel::create_integrator_state(),
            band_pass_voltage: 0,
            cutoff_register: 0,
            cutoff_voltage_squared: 0,
            high_pass_voltage: 0,
            low_pass_voltage: 0,
            resonance_routing: 0,
            mode_volume: 0,
            output_pcm_state: 0,
        };
        result.reset();
        result
    }

    pub const fn cutoff(&self) -> u16 {
        self.cutoff_register
    }

    pub fn set_cutoff(&mut self, cutoff: u16) {
        let normalized = cutoff & 0x07ff;
        if normalized == self.cutoff_register {
            return;
        }
        self.cutoff_voltage_squared = self.model.cutoff_control_voltage_squared(normalized);
        self.cutoff_register = normalized;
    }

    pub const fn resonance_routing(&self) -> u8 {
        self.resonance_routing
    }

    pub fn set_resonance_routing(&mut self, value: u8) {
        self.resonance_routing = value;
    }

    pub const fn mode_volume(&self) -> u8 {
        self.mode_volume
    }

    pub fn set_mode_volume(&mut self, value: u8) {
        self.mode_volume = value;
    }

    pub const fn output_pcm(&self) -> i16 {
        self.output_pcm_state
    }

    pub fn reset(&mut self) {
        Sid6581FilterModel::reset_integrator_state(&mut self.band_pass_integrator);
        Sid6581FilterModel::reset_integrator_state(&mut self.low_pass_integrator);
        self.band_pass_voltage = 0;
        self.high_pass_voltage = 0;
        self.low_pass_voltage = 0;
        self.cutoff_register = 0;
        self.cutoff_voltage_squared = self.model.cutoff_control_voltage_squared(0);
        self.resonance_routing = 0;
        self.mode_volume = 0;
        self.output_pcm_state = 0;
    }

    pub fn clock(
        &mut self,
        voice_1: i32,
        voice_2: i32,
        voice_3: i32,
        external_input: Option<i32>,
    ) -> i16 {
        let voice_1_voltage = self.model.scale_voice(voice_1);
        let voice_2_voltage = self.model.scale_voice(voice_2);
        let voice_3_voltage = self.model.scale_voice(voice_3);
        let mut filter_input_count = 0_usize;
        let mut filter_input_voltage = 0_i32;
        let mut mixer_input_count = 0_usize;
        let mut mixer_input_voltage = 0_i32;

        route_input(
            self.resonance_routing & filter_bit::VOICE_1 != 0,
            voice_1_voltage,
            &mut filter_input_count,
            &mut filter_input_voltage,
            &mut mixer_input_count,
            &mut mixer_input_voltage,
        );
        route_input(
            self.resonance_routing & filter_bit::VOICE_2 != 0,
            voice_2_voltage,
            &mut filter_input_count,
            &mut filter_input_voltage,
            &mut mixer_input_count,
            &mut mixer_input_voltage,
        );
        if self.resonance_routing & filter_bit::VOICE_3 != 0 {
            filter_input_count += 1;
            filter_input_voltage += voice_3_voltage;
        } else if self.mode_volume & filter_bit::MUTE_VOICE_3 == 0 {
            mixer_input_count += 1;
            mixer_input_voltage += voice_3_voltage;
        }
        if let Some(external_input) = external_input {
            route_input(
                self.resonance_routing & filter_bit::EXTERNAL_INPUT != 0,
                self.model.scale_external_input(external_input),
                &mut filter_input_count,
                &mut filter_input_voltage,
                &mut mixer_input_count,
                &mut mixer_input_voltage,
            );
        }

        self.low_pass_voltage = self.model.integrate(
            self.band_pass_voltage,
            &mut self.low_pass_integrator,
            self.cutoff_voltage_squared,
        );
        self.band_pass_voltage = self.model.integrate(
            self.high_pass_voltage,
            &mut self.band_pass_integrator,
            self.cutoff_voltage_squared,
        );
        let resonance = (self.resonance_routing >> 4) & 0x0f;
        let resonance_voltage = self.model.resonance_gain(resonance, self.band_pass_voltage);
        self.high_pass_voltage = self.model.sum_filter_inputs(
            filter_input_count,
            resonance_voltage + self.low_pass_voltage + filter_input_voltage,
        );

        if self.mode_volume & filter_bit::LOW_PASS != 0 {
            mixer_input_count += 1;
            mixer_input_voltage += self.low_pass_voltage;
        }
        if self.mode_volume & filter_bit::BAND_PASS != 0 {
            mixer_input_count += 1;
            mixer_input_voltage += self.band_pass_voltage;
        }
        if self.mode_volume & filter_bit::HIGH_PASS != 0 {
            mixer_input_count += 1;
            mixer_input_voltage += self.high_pass_voltage;
        }

        let mixed_voltage = self
            .model
            .mix_audio_inputs(mixer_input_count, mixer_input_voltage);
        self.output_pcm_state = self
            .model
            .apply_volume(self.mode_volume & 0x0f, mixed_voltage);
        self.output_pcm_state
    }
}

#[allow(clippy::too_many_arguments)]
fn route_input(
    filtered: bool,
    voltage: i32,
    filter_input_count: &mut usize,
    filter_input_voltage: &mut i32,
    mixer_input_count: &mut usize,
    mixer_input_voltage: &mut i32,
) {
    if filtered {
        *filter_input_count += 1;
        *filter_input_voltage += voltage;
    } else {
        *mixer_input_count += 1;
        *mixer_input_voltage += voltage;
    }
}

#[cfg(test)]
mod tests {
    use super::SidMos6581Filter;
    use crate::devices::sid::filter_bit;

    #[test]
    fn reproduces_the_nonlinear_low_pass_impulse() {
        let mut filter = SidMos6581Filter::new();
        let expected = [
            -3_271, -3_271, -3_289, -3_318, -3_357, -3_406, -3_468, -3_536, -3_608, -3_689, -3_782,
            -3_878,
        ];
        filter.set_cutoff(0x0640);
        filter.set_resonance_routing(0xa0 | filter_bit::VOICE_1);
        filter.set_mode_volume(filter_bit::LOW_PASS | 0x0f);

        let actual: Vec<i16> = expected
            .iter()
            .enumerate()
            .map(|(cycle, _)| filter.clock(if cycle == 0 { 400_000 } else { 0 }, 0, 0, None))
            .collect();

        assert_eq!(actual, expected);
    }
}
