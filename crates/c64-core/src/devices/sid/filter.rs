// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - MOS 8580 fixed-point SID filter
//
//   File:       sid/filter.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use super::{SidModel, SidMos6581Filter, filter_bit};

const SIGNED_PCM_MINIMUM: i64 = i16::MIN as i64;
const SIGNED_PCM_MAXIMUM: i64 = i16::MAX as i64;
const MOS8580_ANGULAR_FREQUENCY_SCALE: i64 = 82_355;
const MOS8580_RECIPROCAL_Q_SCALED: [i64; 16] = [
    1_448, 1_328, 1_218, 1_117, 1_024, 939, 861, 790, 724, 664, 609, 558, 512, 470, 431, 395,
];
const MOS8580_RECIPROCAL_Q_FRACTION_BITS: u32 = 10;
const MOS8580_VOICE_SCALE: i64 = 2_152;
const MOS8580_VOICE_SCALE_FRACTION_BITS: u32 = 18;
const FILTER_CUTOFF_MASK: u16 = 0x07ff;

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
enum SidFilterBackend {
    Mos6581(SidMos6581Filter),
    Mos8580(SidMos8580Filter),
}

/// Model-selecting SID internal filter facade with one register contract for
/// both chip revisions.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct SidFilter {
    backend: SidFilterBackend,
}

impl SidFilter {
    pub fn new(model: SidModel) -> Self {
        Self {
            backend: match model {
                SidModel::Mos6581 => SidFilterBackend::Mos6581(SidMos6581Filter::new()),
                SidModel::Mos8580 => SidFilterBackend::Mos8580(SidMos8580Filter::new()),
            },
        }
    }

    pub const fn model(&self) -> SidModel {
        match self.backend {
            SidFilterBackend::Mos6581(_) => SidModel::Mos6581,
            SidFilterBackend::Mos8580(_) => SidModel::Mos8580,
        }
    }

    pub const fn cutoff(&self) -> u16 {
        match &self.backend {
            SidFilterBackend::Mos6581(filter) => filter.cutoff(),
            SidFilterBackend::Mos8580(filter) => filter.cutoff(),
        }
    }

    pub fn set_cutoff(&mut self, cutoff: u16) {
        match &mut self.backend {
            SidFilterBackend::Mos6581(filter) => filter.set_cutoff(cutoff),
            SidFilterBackend::Mos8580(filter) => filter.set_cutoff(cutoff),
        }
    }

    pub const fn resonance_routing(&self) -> u8 {
        match &self.backend {
            SidFilterBackend::Mos6581(filter) => filter.resonance_routing(),
            SidFilterBackend::Mos8580(filter) => filter.resonance_routing(),
        }
    }

    pub fn set_resonance_routing(&mut self, value: u8) {
        match &mut self.backend {
            SidFilterBackend::Mos6581(filter) => filter.set_resonance_routing(value),
            SidFilterBackend::Mos8580(filter) => filter.set_resonance_routing(value),
        }
    }

    pub const fn mode_volume(&self) -> u8 {
        match &self.backend {
            SidFilterBackend::Mos6581(filter) => filter.mode_volume(),
            SidFilterBackend::Mos8580(filter) => filter.mode_volume(),
        }
    }

    pub fn set_mode_volume(&mut self, value: u8) {
        match &mut self.backend {
            SidFilterBackend::Mos6581(filter) => filter.set_mode_volume(value),
            SidFilterBackend::Mos8580(filter) => filter.set_mode_volume(value),
        }
    }

    pub const fn output_pcm(&self) -> i16 {
        match &self.backend {
            SidFilterBackend::Mos6581(filter) => filter.output_pcm(),
            SidFilterBackend::Mos8580(filter) => filter.output_pcm(),
        }
    }

    pub fn reset(&mut self) {
        match &mut self.backend {
            SidFilterBackend::Mos6581(filter) => filter.reset(),
            SidFilterBackend::Mos8580(filter) => filter.reset(),
        }
    }

    pub fn clock(
        &mut self,
        voice_1: i32,
        voice_2: i32,
        voice_3: i32,
        external_input: Option<i32>,
    ) -> i16 {
        match &mut self.backend {
            SidFilterBackend::Mos6581(filter) => {
                filter.clock(voice_1, voice_2, voice_3, external_input)
            }
            SidFilterBackend::Mos8580(filter) => {
                filter.clock(voice_1, voice_2, voice_3, external_input)
            }
        }
    }
}

/// Linear two-integrator-loop model of the revised MOS 8580 filter and mixer.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct SidMos8580Filter {
    cutoff: u16,
    resonance_routing: u8,
    mode_volume: u8,
    low_pass_state: i64,
    band_pass_state: i64,
    high_pass_state: i64,
    output_pcm_state: i16,
}

impl Default for SidMos8580Filter {
    fn default() -> Self {
        Self::new()
    }
}

impl SidMos8580Filter {
    pub const fn new() -> Self {
        Self {
            cutoff: 0,
            resonance_routing: 0,
            mode_volume: 0,
            low_pass_state: 0,
            band_pass_state: 0,
            high_pass_state: 0,
            output_pcm_state: 0,
        }
    }

    pub const fn cutoff(&self) -> u16 {
        self.cutoff
    }

    pub fn set_cutoff(&mut self, cutoff: u16) {
        self.cutoff = cutoff & FILTER_CUTOFF_MASK;
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
        *self = Self::new();
    }

    pub fn clock(
        &mut self,
        voice_1: i32,
        voice_2: i32,
        voice_3: i32,
        external_input: Option<i32>,
    ) -> i16 {
        let scaled_voice_1 = scale_voice(voice_1);
        let scaled_voice_2 = scale_voice(voice_2);
        let scaled_voice_3 = scale_voice(voice_3);
        let mut filtered_input = 0_i64;
        let mut direct_output = 0_i64;
        route_input(
            self.resonance_routing & filter_bit::VOICE_1 != 0,
            scaled_voice_1,
            &mut filtered_input,
            &mut direct_output,
        );
        route_input(
            self.resonance_routing & filter_bit::VOICE_2 != 0,
            scaled_voice_2,
            &mut filtered_input,
            &mut direct_output,
        );
        if self.resonance_routing & filter_bit::VOICE_3 != 0 {
            filtered_input += scaled_voice_3;
        } else if self.mode_volume & filter_bit::MUTE_VOICE_3 == 0 {
            direct_output += scaled_voice_3;
        }
        if let Some(external_input) = external_input {
            route_input(
                self.resonance_routing & filter_bit::EXTERNAL_INPUT != 0,
                i64::from(external_input),
                &mut filtered_input,
                &mut direct_output,
            );
        }

        let angular_frequency =
            (MOS8580_ANGULAR_FREQUENCY_SCALE * i64::from(self.cutoff + 1)) >> 11;
        let band_pass_delta = (angular_frequency * (self.high_pass_state >> 4)) >> 16;
        let low_pass_delta = (angular_frequency * (self.band_pass_state >> 4)) >> 16;
        self.band_pass_state -= band_pass_delta;
        self.low_pass_state -= low_pass_delta;

        let resonance = usize::from((self.resonance_routing >> 4) & 0x0f);
        let reciprocal_q = MOS8580_RECIPROCAL_Q_SCALED[resonance];
        self.high_pass_state = ((self.band_pass_state * reciprocal_q)
            >> MOS8580_RECIPROCAL_Q_FRACTION_BITS)
            - self.low_pass_state
            - filtered_input;

        let mut mixed = direct_output;
        if self.mode_volume & filter_bit::LOW_PASS != 0 {
            mixed += self.low_pass_state;
        }
        if self.mode_volume & filter_bit::BAND_PASS != 0 {
            mixed += self.band_pass_state;
        }
        if self.mode_volume & filter_bit::HIGH_PASS != 0 {
            mixed += self.high_pass_state;
        }

        let volume = i64::from(self.mode_volume & 0x0f);
        let output = ((mixed * volume) >> 4).clamp(SIGNED_PCM_MINIMUM, SIGNED_PCM_MAXIMUM);
        self.output_pcm_state =
            i16::try_from(output).unwrap_or(if output < 0 { i16::MIN } else { i16::MAX });
        self.output_pcm_state
    }
}

fn scale_voice(value: i32) -> i64 {
    (i64::from(value) * MOS8580_VOICE_SCALE) >> MOS8580_VOICE_SCALE_FRACTION_BITS
}

fn route_input(filtered: bool, value: i64, filtered_input: &mut i64, direct_output: &mut i64) {
    if filtered {
        *filtered_input += value;
    } else {
        *direct_output += value;
    }
}

#[cfg(test)]
mod tests {
    use super::SidMos8580Filter;
    use crate::devices::sid::filter_bit;

    #[test]
    fn routed_voice_three_is_not_muted() {
        let voice_3 = 2_000 * 0xff;
        let mut direct = SidMos8580Filter::new();
        direct.set_mode_volume(filter_bit::MUTE_VOICE_3 | 0x0f);
        assert_eq!(direct.clock(0, 0, voice_3, None), 0);

        let mut routed = SidMos8580Filter::new();
        routed.set_cutoff(0x07ff);
        routed.set_resonance_routing(filter_bit::VOICE_3);
        routed.set_mode_volume(filter_bit::MUTE_VOICE_3 | filter_bit::HIGH_PASS | 0x0f);
        assert_ne!(routed.clock(0, 0, voice_3, None), 0);
    }

    #[test]
    fn reset_clears_registers_and_integrators() {
        let mut filter = SidMos8580Filter::new();
        filter.set_cutoff(0x0500);
        filter.set_resonance_routing(0xf1);
        filter.set_mode_volume(filter_bit::LOW_PASS | 0x0f);
        for _ in 0..100 {
            filter.clock(300_000, 0, 0, None);
        }

        filter.reset();

        assert_eq!(filter.cutoff(), 0);
        assert_eq!(filter.resonance_routing(), 0);
        assert_eq!(filter.mode_volume(), 0);
        assert_eq!(filter.output_pcm(), 0);
        assert_eq!(filter.clock(0, 0, 0, None), 0);
    }
}
