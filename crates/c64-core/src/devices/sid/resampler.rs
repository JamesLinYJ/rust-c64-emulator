// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic SID area resampler
//
//   File:       sid/resampler.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

const SIGNED_PCM_SCALE: f64 = 32_768.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SidResamplerConfigError {
    ZeroInputRate,
    ZeroOutputRate,
    Upsampling {
        input_rate_hz: u32,
        output_rate_hz: u32,
    },
}

impl fmt::Display for SidResamplerConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInputRate => formatter.write_str("SID resampler input rate must be positive"),
            Self::ZeroOutputRate => {
                formatter.write_str("SID resampler output rate must be positive")
            }
            Self::Upsampling {
                input_rate_hz,
                output_rate_hz,
            } => write!(
                formatter,
                "SID area resampler does not support upsampling {input_rate_hz} Hz to {output_rate_hz} Hz"
            ),
        }
    }
}

impl std::error::Error for SidResamplerConfigError {}

/// Exact-area downsampler. Phase, area and interval lengths remain integer;
/// only a completed normalized PCM sample is converted to `f32`.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct SidAudioResampler {
    input_rate_hz: u32,
    output_rate_hz: u32,
    accumulated_area: i64,
    accumulated_weight: u32,
    weight_until_output: u32,
}

impl SidAudioResampler {
    /// # Errors
    ///
    /// Returns an error for zero rates or when upsampling is requested.
    pub fn new(input_rate_hz: u32, output_rate_hz: u32) -> Result<Self, SidResamplerConfigError> {
        if input_rate_hz == 0 {
            return Err(SidResamplerConfigError::ZeroInputRate);
        }
        if output_rate_hz == 0 {
            return Err(SidResamplerConfigError::ZeroOutputRate);
        }
        if output_rate_hz > input_rate_hz {
            return Err(SidResamplerConfigError::Upsampling {
                input_rate_hz,
                output_rate_hz,
            });
        }
        Ok(Self::new_with_valid_rates(input_rate_hz, output_rate_hz))
    }

    pub(crate) fn new_with_valid_rates(input_rate_hz: u32, output_rate_hz: u32) -> Self {
        debug_assert_ne!(input_rate_hz, 0);
        debug_assert_ne!(output_rate_hz, 0);
        debug_assert!(output_rate_hz <= input_rate_hz);
        Self {
            input_rate_hz,
            output_rate_hz,
            accumulated_area: 0,
            accumulated_weight: 0,
            weight_until_output: input_rate_hz,
        }
    }

    pub const fn input_rate_hz(&self) -> u32 {
        self.input_rate_hz
    }

    pub const fn output_rate_hz(&self) -> u32 {
        self.output_rate_hz
    }

    pub fn reset(&mut self) {
        self.accumulated_area = 0;
        self.accumulated_weight = 0;
        self.weight_until_output = self.input_rate_hz;
    }

    pub fn push_pcm(&mut self, input_pcm: i32) -> Option<f32> {
        if self.output_rate_hz < self.weight_until_output {
            self.accumulated_area += i64::from(input_pcm) * i64::from(self.output_rate_hz);
            self.accumulated_weight += self.output_rate_hz;
            self.weight_until_output -= self.output_rate_hz;
            return None;
        }

        let mut input_weight_remaining = self.output_rate_hz;
        let mut output = None;
        while input_weight_remaining > 0 {
            let consumed_weight = input_weight_remaining.min(self.weight_until_output);
            self.accumulated_area += i64::from(input_pcm) * i64::from(consumed_weight);
            self.accumulated_weight += consumed_weight;
            input_weight_remaining -= consumed_weight;
            self.weight_until_output -= consumed_weight;

            if self.weight_until_output == 0 {
                debug_assert_eq!(self.accumulated_weight, self.input_rate_hz);
                output = Some(normalize_area(
                    self.accumulated_area,
                    self.accumulated_weight,
                ));
                self.accumulated_area = 0;
                self.accumulated_weight = 0;
                self.weight_until_output = self.input_rate_hz;
            }
        }
        output
    }
}

// A completed interval is bounded by i16::MAX * u32::MAX, below 2^47 for
// supported SID clocks, so conversion to f64 is exact. Conversion to f32 is
// the intentional public audio-buffer quantization boundary.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn normalize_area(accumulated_area: i64, accumulated_weight: u32) -> f32 {
    let normalized_area = accumulated_area as f64 / SIGNED_PCM_SCALE;
    (normalized_area / f64::from(accumulated_weight)) as f32
}

#[cfg(test)]
mod tests {
    use super::{SidAudioResampler, normalize_area};

    #[test]
    fn preserves_constant_pcm_at_a_non_integer_ratio() {
        let mut resampler = SidAudioResampler::new(985_248, 44_100).expect("valid rates");
        let mut output = Vec::new();
        for _ in 0..985_248 {
            if let Some(sample) = resampler.push_pcm(12_288) {
                output.push(sample);
            }
        }

        assert_eq!(output.len(), 44_100);
        assert!(
            output
                .iter()
                .all(|sample| sample.to_bits() == 0.375_f32.to_bits())
        );
    }

    #[test]
    fn weights_a_source_cycle_split_by_an_output_boundary() {
        let mut resampler = SidAudioResampler::new(5, 2).expect("valid rates");
        let output: Vec<f32> = [i16::MAX, i16::MAX, 0, 0, 0]
            .into_iter()
            .filter_map(|sample| resampler.push_pcm(i32::from(sample)))
            .collect();

        assert_eq!(output.len(), 2);
        let expected = normalize_area(i64::from(i16::MAX) * 4, 5);
        assert_eq!(output[0].to_bits(), expected.to_bits());
        assert_eq!(output[1].to_bits(), 0_f32.to_bits());
    }

    #[test]
    fn rejects_upsampling() {
        assert!(SidAudioResampler::new(44_100, 48_000).is_err());
    }
}
