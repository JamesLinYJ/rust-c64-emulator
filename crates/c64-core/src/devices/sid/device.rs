// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - complete MOS 6581/8580 SID device
//
//   File:       sid/device.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use super::sample_buffer::SidSampleBuffer;
use super::{
    SID_REGISTER_COUNT, SID_SAMPLE_BUFFER_CAPACITY, SID_VOICE_COUNT, SID_VOICE_REGISTER_COUNT,
    SidAudioResampler, SidExternalFilter, SidFilter, SidModel, SidVoice, register, voice_register,
};

pub const PAL_PROCESSOR_CLOCK_HZ: u32 = 985_248;
pub const NTSC_PROCESSOR_CLOCK_HZ: u32 = 1_022_727;
pub const DEFAULT_SAMPLE_RATE_HZ: u32 = 44_100;
const MOS6581_BUS_LATCH_DECAY_CYCLES: u32 = 0x1d00;
const MOS8580_BUS_LATCH_DECAY_CYCLES: u32 = 0x0a_2000;
const VOICE_REGISTER_COUNT: u8 = 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SidConfigError {
    ZeroProcessorClock,
    ZeroSampleRate,
    Upsampling {
        processor_clock_hz: u32,
        sample_rate_hz: u32,
    },
}

impl fmt::Display for SidConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroProcessorClock => formatter.write_str("SID processor clock must be positive"),
            Self::ZeroSampleRate => formatter.write_str("SID sample rate must be positive"),
            Self::Upsampling {
                processor_clock_hz,
                sample_rate_hz,
            } => write!(
                formatter,
                "SID area resampler does not support upsampling {processor_clock_hz} Hz to {sample_rate_hz} Hz"
            ),
        }
    }
}

impl std::error::Error for SidConfigError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SidVoiceState {
    pub attack_decay: u8,
    pub control: u8,
    pub envelope: u8,
    pub frequency: u16,
    pub pulse_width: u16,
    pub sustain_release: u8,
}

/// Cycle-clocked SID including digital voices, analog filters, readable bus
/// behavior and a fixed-capacity resampled PCM queue.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Sid {
    registers: [u8; SID_REGISTER_COUNT],
    voices: [SidVoice; SID_VOICE_COUNT],
    filter: SidFilter,
    external_filter: SidExternalFilter,
    resampler: SidAudioResampler,
    samples: SidSampleBuffer,
    model: SidModel,
    processor_clock_hz: u32,
    sample_rate_hz: u32,
    bus_latch: u8,
    bus_latch_cycles_remaining: u32,
    paddle_x: u8,
    paddle_y: u8,
}

impl Default for Sid {
    fn default() -> Self {
        Self::new_with_valid_rates(
            SidModel::Mos6581,
            PAL_PROCESSOR_CLOCK_HZ,
            DEFAULT_SAMPLE_RATE_HZ,
        )
    }
}

impl Sid {
    /// # Errors
    ///
    /// Returns an error for zero rates or a sample rate above the chip clock.
    pub fn new(
        model: SidModel,
        processor_clock_hz: u32,
        sample_rate_hz: u32,
    ) -> Result<Self, SidConfigError> {
        if processor_clock_hz == 0 {
            return Err(SidConfigError::ZeroProcessorClock);
        }
        if sample_rate_hz == 0 {
            return Err(SidConfigError::ZeroSampleRate);
        }
        if sample_rate_hz > processor_clock_hz {
            return Err(SidConfigError::Upsampling {
                processor_clock_hz,
                sample_rate_hz,
            });
        }
        Ok(Self::new_with_valid_rates(
            model,
            processor_clock_hz,
            sample_rate_hz,
        ))
    }

    pub(crate) fn new_with_valid_rates(
        model: SidModel,
        processor_clock_hz: u32,
        sample_rate_hz: u32,
    ) -> Self {
        debug_assert_ne!(processor_clock_hz, 0);
        debug_assert_ne!(sample_rate_hz, 0);
        debug_assert!(sample_rate_hz <= processor_clock_hz);
        let mut result = Self {
            registers: [0; SID_REGISTER_COUNT],
            voices: std::array::from_fn(|_| SidVoice::new(model)),
            filter: SidFilter::new(model),
            external_filter: SidExternalFilter::new_with_valid_clock(processor_clock_hz),
            resampler: SidAudioResampler::new_with_valid_rates(processor_clock_hz, sample_rate_hz),
            samples: SidSampleBuffer::new(SID_SAMPLE_BUFFER_CAPACITY),
            model,
            processor_clock_hz,
            sample_rate_hz,
            bus_latch: 0,
            bus_latch_cycles_remaining: 0,
            paddle_x: u8::MAX,
            paddle_y: u8::MAX,
        };
        result.reset();
        result
    }

    pub const fn model(&self) -> SidModel {
        self.model
    }

    pub const fn processor_clock_hz(&self) -> u32 {
        self.processor_clock_hz
    }

    pub const fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    pub const fn master_volume(&self) -> u8 {
        self.registers[register::FILTER_MODE_VOLUME as usize] & 0x0f
    }

    pub const fn filter_cutoff(&self) -> u16 {
        self.filter.cutoff()
    }

    pub const fn pending_sample_count(&self) -> usize {
        self.samples.len()
    }

    pub(crate) fn state_is_valid(&self) -> bool {
        self.processor_clock_hz > 0
            && self.sample_rate_hz > 0
            && self.sample_rate_hz <= self.processor_clock_hz
            && self.filter.model() == self.model
            && self.voices.iter().all(|voice| voice.model() == self.model)
            && self.resampler.input_rate_hz() == self.processor_clock_hz
            && self.resampler.output_rate_hz() == self.sample_rate_hz
            && self.samples.state_is_valid(SID_SAMPLE_BUFFER_CAPACITY)
    }

    pub fn voice_state(&self, voice: usize) -> Option<SidVoiceState> {
        self.voices.get(voice).map(|state| SidVoiceState {
            frequency: state.frequency(),
            pulse_width: state.pulse_width(),
            control: state.control(),
            attack_decay: state.attack_decay(),
            sustain_release: state.sustain_release(),
            envelope: state.envelope_output(),
        })
    }

    pub fn reset(&mut self) {
        self.registers.fill(0);
        for voice in &mut self.voices {
            voice.reset();
        }
        self.filter.reset();
        self.external_filter.reset();
        self.resampler.reset();
        self.samples.clear();
        self.bus_latch = 0;
        self.bus_latch_cycles_remaining = 0;
        self.paddle_x = u8::MAX;
        self.paddle_y = u8::MAX;
    }

    pub fn clock_cycles(&mut self, cycles: u32) {
        if cycles != 0 && self.silent_clock_is_batchable() {
            self.clock_silent_cycles(cycles);
            return;
        }
        for _ in 0..cycles {
            self.clock_cycle();
        }
    }

    /// Clock one complete SID chip cycle without crossing any host boundary.
    pub fn clock_cycle(&mut self) {
        let oscillators_quiescent = self
            .voices
            .iter()
            .all(SidVoice::oscillator_clock_is_quiescent);
        for voice in &mut self.voices {
            voice.clock_envelope();
            if !oscillators_quiescent {
                voice.clock_oscillator();
            }
        }

        if !oscillators_quiescent {
            self.synchronize_oscillators();
        }

        let filter_output = self.filter.clock(
            self.voices[0].analog_output(),
            self.voices[1].analog_output(),
            self.voices[2].analog_output(),
            None,
        );
        let board_output = self.external_filter.clock_pcm(filter_output);
        let board_output = i32::try_from(board_output).unwrap_or(if board_output < 0 {
            i32::MIN
        } else {
            i32::MAX
        });
        if let Some(sample) = self.resampler.push_pcm(board_output) {
            self.samples.push(sample);
        }
        if self.bus_latch_cycles_remaining != 0 {
            self.bus_latch_cycles_remaining -= 1;
        }
    }

    pub fn read(&mut self, address: u16) -> u8 {
        match address.to_le_bytes()[0] & 0x1f {
            register::PADDLE_X => self.read_driven_value(self.paddle_x),
            register::PADDLE_Y => self.read_driven_value(self.paddle_y),
            register::OSCILLATOR_3 => self.read_driven_value(self.voices[2].oscillator_readback()),
            register::ENVELOPE_3 => self.read_driven_value(self.voices[2].envelope_readback()),
            _ => self.read_bus_latch(),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        self.latch_bus(value);
        let index = address.to_le_bytes()[0] & 0x1f;
        if index >= register::PADDLE_X {
            return;
        }
        self.registers[usize::from(index)] = value;
        if usize::from(index) < SID_VOICE_COUNT * SID_VOICE_REGISTER_COUNT {
            self.update_voice_register(index);
        } else {
            self.update_filter_registers();
        }
    }

    pub fn set_paddle_inputs(&mut self, x: u8, y: u8) {
        self.paddle_x = x;
        self.paddle_y = y;
    }

    pub fn drain_samples(&mut self, maximum_length: Option<usize>) -> Vec<f32> {
        self.samples.drain(maximum_length.unwrap_or(usize::MAX))
    }

    pub fn pull_samples_into(&mut self, destination: &mut [f32]) -> usize {
        self.samples.pull_into(destination)
    }

    fn update_voice_register(&mut self, index: u8) {
        let voice_index = usize::from(index / VOICE_REGISTER_COUNT);
        let register_index = index % VOICE_REGISTER_COUNT;
        let base = voice_index * SID_VOICE_REGISTER_COUNT;
        match register_index {
            voice_register::FREQUENCY_LOW | voice_register::FREQUENCY_HIGH => {
                let frequency =
                    u16::from(self.registers[base + voice_register::FREQUENCY_LOW as usize])
                        | (u16::from(
                            self.registers[base + voice_register::FREQUENCY_HIGH as usize],
                        ) << 8);
                self.voices[voice_index].set_frequency(frequency);
            }
            voice_register::PULSE_WIDTH_LOW | voice_register::PULSE_WIDTH_HIGH => {
                let pulse_width =
                    u16::from(self.registers[base + voice_register::PULSE_WIDTH_LOW as usize])
                        | (u16::from(
                            self.registers[base + voice_register::PULSE_WIDTH_HIGH as usize],
                        ) << 8);
                self.voices[voice_index].set_pulse_width(pulse_width);
            }
            voice_register::CONTROL => {
                let source_accumulator = self.voices[source_index(voice_index)].accumulator();
                self.voices[voice_index].set_control_with_sync_source(
                    self.registers[usize::from(index)],
                    source_accumulator,
                );
            }
            voice_register::ATTACK_DECAY => {
                self.voices[voice_index].set_attack_decay(self.registers[usize::from(index)]);
            }
            voice_register::SUSTAIN_RELEASE => {
                self.voices[voice_index].set_sustain_release(self.registers[usize::from(index)]);
            }
            _ => {}
        }
    }

    fn synchronize_oscillators(&mut self) {
        let rising_0 = self.voices[0].oscillator_msb_rising();
        let rising_1 = self.voices[1].oscillator_msb_rising();
        let rising_2 = self.voices[2].oscillator_msb_rising();
        let sync_0 = self.voices[0].oscillator_sync_enabled();
        let sync_1 = self.voices[1].oscillator_sync_enabled();
        let sync_2 = self.voices[2].oscillator_sync_enabled();
        if rising_2 && sync_0 && !(sync_2 && rising_1) {
            self.voices[0].reset_accumulator_for_sync();
        }
        if rising_0 && sync_1 && !(sync_0 && rising_2) {
            self.voices[1].reset_accumulator_for_sync();
        }
        if rising_1 && sync_2 && !(sync_1 && rising_0) {
            self.voices[2].reset_accumulator_for_sync();
        }

        let accumulator_0 = self.voices[0].accumulator();
        let accumulator_1 = self.voices[1].accumulator();
        let accumulator_2 = self.voices[2].accumulator();
        self.voices[0].update_waveform_output_with_sync_source(accumulator_2);
        self.voices[1].update_waveform_output_with_sync_source(accumulator_0);
        self.voices[2].update_waveform_output_with_sync_source(accumulator_1);
    }

    fn silent_clock_is_batchable(&self) -> bool {
        self.voices.iter().all(SidVoice::silent_clock_is_batchable)
            && self.filter.zero_input_is_stationary()
            && self
                .external_filter
                .constant_input_is_stationary(self.filter.output_pcm())
    }

    fn clock_silent_cycles(&mut self, cycles: u32) {
        debug_assert!(self.silent_clock_is_batchable());
        for voice in &mut self.voices {
            voice.clock_silent_cycles(cycles);
        }
        let (resampler, samples) = (&mut self.resampler, &mut self.samples);
        resampler.push_zero_pcm_cycles(cycles, |sample| {
            samples.push(sample);
        });
        self.bus_latch_cycles_remaining = self.bus_latch_cycles_remaining.saturating_sub(cycles);
    }

    fn update_filter_registers(&mut self) {
        let cutoff = (u16::from(self.registers[register::FILTER_CUTOFF_HIGH as usize]) << 3)
            | u16::from(self.registers[register::FILTER_CUTOFF_LOW as usize] & 0x07);
        self.filter.set_cutoff(cutoff);
        self.filter
            .set_resonance_routing(self.registers[register::FILTER_RESONANCE_ROUTING as usize]);
        self.filter
            .set_mode_volume(self.registers[register::FILTER_MODE_VOLUME as usize]);
    }

    fn latch_bus(&mut self, value: u8) {
        self.bus_latch = value;
        self.bus_latch_cycles_remaining = match self.model {
            SidModel::Mos6581 => MOS6581_BUS_LATCH_DECAY_CYCLES,
            SidModel::Mos8580 => MOS8580_BUS_LATCH_DECAY_CYCLES,
        };
    }

    const fn read_bus_latch(&self) -> u8 {
        if self.bus_latch_cycles_remaining > 0 {
            self.bus_latch
        } else {
            0
        }
    }

    fn read_driven_value(&mut self, value: u8) -> u8 {
        self.latch_bus(value);
        value
    }
}

const fn source_index(index: usize) -> usize {
    match index {
        0 => 2,
        1 => 0,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{Sid, SidModel};
    use crate::devices::sid::{control, filter_bit, register};

    #[test]
    fn maps_voice_registers_and_clocks_adsr() {
        let mut sid = Sid::default();
        sid.write(0x00, 0x34);
        sid.write(0x01, 0x12);
        sid.write(0x02, 0xcd);
        sid.write(0x03, 0x0a);
        sid.write(0x05, 0x00);
        sid.write(0x06, 0xf0);
        let power_on = sid.voice_state(0).expect("voice exists").envelope;
        sid.write(0x04, control::GATE | control::SAWTOOTH);
        sid.clock_cycles(90);
        let voice = sid.voice_state(0).expect("voice exists");

        assert_eq!(voice.frequency, 0x1234);
        assert_eq!(voice.pulse_width, 0x0acd);
        assert!(voice.envelope > power_on);
    }

    #[test]
    fn models_bus_latch_decay_and_readable_paddles() {
        let mut sid = Sid::default();
        sid.write(0, 0x73);
        assert_eq!(sid.read(0), 0x73);
        sid.clock_cycles(0x1cff);
        assert_eq!(sid.read(0), 0x73);
        sid.clock_cycle();
        assert_eq!(sid.read(0), 0);

        sid.set_paddle_inputs(0x12, 0x34);
        assert_eq!(sid.read(u16::from(register::PADDLE_X)), 0x12);
        assert_eq!(sid.read(u16::from(register::PADDLE_Y)), 0x34);
    }

    #[test]
    fn produces_bounded_resampled_audio_for_both_models() {
        for model in [SidModel::Mos6581, SidModel::Mos8580] {
            let mut sid = Sid::new(model, 100_000, 10_000).expect("valid rates");
            sid.write(0x00, 0xff);
            sid.write(0x01, 0x7f);
            sid.write(0x05, 0x00);
            sid.write(0x06, 0xf0);
            sid.write(0x04, control::GATE | control::SAWTOOTH);
            sid.write(
                u16::from(register::FILTER_MODE_VOLUME),
                filter_bit::LOW_PASS | 0x0f,
            );
            sid.clock_cycles(2_000);
            let samples = sid.drain_samples(None);

            assert_eq!(samples.len(), 200);
            assert!(samples.iter().any(|sample| *sample != 0.0));
            assert!(
                samples
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
            );
        }
    }

    #[test]
    fn batched_stable_silence_matches_individual_chip_cycles() {
        let mut initial = Sid::default();
        initial.clock_cycles(120_000);
        initial.drain_samples(None);
        assert!(initial.silent_clock_is_batchable());
        let mut stepped = initial.clone();
        let mut batched = initial;

        for _ in 0..20_000 {
            stepped.clock_cycle();
        }
        batched.clock_cycles(20_000);

        assert_eq!(batched, stepped);
    }
}
