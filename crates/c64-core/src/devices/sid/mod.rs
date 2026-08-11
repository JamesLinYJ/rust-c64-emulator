// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - MOS 6581/8580 SID subsystem
//
//   File:       sid/mod.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

mod device;
mod envelope;
mod external_filter;
mod filter;
mod filter_6581;
mod filter_6581_model;
mod oscillator;
mod resampler;
mod sample_buffer;
mod voice;
mod waveform_data;

pub use device::{
    DEFAULT_SAMPLE_RATE_HZ, NTSC_PROCESSOR_CLOCK_HZ, PAL_PROCESSOR_CLOCK_HZ, Sid, SidConfigError,
    SidVoiceState,
};
pub use envelope::SidEnvelopeGenerator;
pub use external_filter::{SidExternalFilter, SidPcmRangeError, SidTimingError};
pub use filter::{SidFilter, SidMos8580Filter};
pub use filter_6581::SidMos6581Filter;
pub use oscillator::SidOscillator;
pub use resampler::{SidAudioResampler, SidResamplerConfigError};
pub use voice::SidVoice;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SidModel {
    #[default]
    Mos6581,
    Mos8580,
}

impl SidModel {
    pub const fn code(self) -> u8 {
        match self {
            Self::Mos6581 => 0,
            Self::Mos8580 => 1,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Mos6581),
            1 => Some(Self::Mos8580),
            _ => None,
        }
    }
}

pub const SID_REGISTER_COUNT: usize = 0x20;
pub const SID_VOICE_COUNT: usize = 3;
pub const SID_VOICE_REGISTER_COUNT: usize = 7;
pub const SID_SAMPLE_BUFFER_CAPACITY: usize = 16_384;

pub mod register {
    pub const FILTER_CUTOFF_LOW: u8 = 0x15;
    pub const FILTER_CUTOFF_HIGH: u8 = 0x16;
    pub const FILTER_RESONANCE_ROUTING: u8 = 0x17;
    pub const FILTER_MODE_VOLUME: u8 = 0x18;
    pub const PADDLE_X: u8 = 0x19;
    pub const PADDLE_Y: u8 = 0x1a;
    pub const OSCILLATOR_3: u8 = 0x1b;
    pub const ENVELOPE_3: u8 = 0x1c;
}

pub mod voice_register {
    pub const FREQUENCY_LOW: u8 = 0;
    pub const FREQUENCY_HIGH: u8 = 1;
    pub const PULSE_WIDTH_LOW: u8 = 2;
    pub const PULSE_WIDTH_HIGH: u8 = 3;
    pub const CONTROL: u8 = 4;
    pub const ATTACK_DECAY: u8 = 5;
    pub const SUSTAIN_RELEASE: u8 = 6;
}

pub mod filter_bit {
    pub const VOICE_1: u8 = 1 << 0;
    pub const VOICE_2: u8 = 1 << 1;
    pub const VOICE_3: u8 = 1 << 2;
    pub const EXTERNAL_INPUT: u8 = 1 << 3;
    pub const LOW_PASS: u8 = 1 << 4;
    pub const BAND_PASS: u8 = 1 << 5;
    pub const HIGH_PASS: u8 = 1 << 6;
    pub const MUTE_VOICE_3: u8 = 1 << 7;
}

pub mod control {
    pub const GATE: u8 = 1 << 0;
    pub const SYNCHRONIZE: u8 = 1 << 1;
    pub const RING_MODULATION: u8 = 1 << 2;
    pub const TEST: u8 = 1 << 3;
    pub const TRIANGLE: u8 = 1 << 4;
    pub const SAWTOOTH: u8 = 1 << 5;
    pub const PULSE: u8 = 1 << 6;
    pub const NOISE: u8 = 1 << 7;
}

pub const ENVELOPE_RATE_COMPARE_VALUES: [u16; 16] = [
    8, 31, 62, 94, 148, 219, 266, 312, 391, 976, 1_953, 3_125, 3_906, 11_719, 19_531, 31_250,
];
