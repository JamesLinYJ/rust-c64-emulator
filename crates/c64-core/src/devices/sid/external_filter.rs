// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - C64 board-level SID output filter
//
//   File:       sid/external_filter.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

const STATE_FRACTION_BITS: u32 = 11;
const STATE_SCALE: i64 = 1 << STATE_FRACTION_BITS;
const LOW_PASS_COEFFICIENT_BITS: u32 = 7;
const HIGH_PASS_COEFFICIENT_BITS: u32 = 17;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SidTimingError;

impl fmt::Display for SidTimingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SID clock frequency must be positive")
    }
}

impl std::error::Error for SidTimingError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SidPcmRangeError {
    value: i32,
}

impl SidPcmRangeError {
    pub const fn value(self) -> i32 {
        self.value
    }
}

impl fmt::Display for SidPcmRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SID external-filter input must be signed 16-bit PCM: {}",
            self.value
        )
    }
}

impl std::error::Error for SidPcmRangeError {}

/// Fixed-point model of the 10 kOhm/1000 pF low-pass and 1 kOhm/10 uF
/// high-pass network connected after SID AUDIO OUT on the C64 board.
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct SidExternalFilter {
    low_pass_coefficient: i64,
    high_pass_coefficient: i64,
    low_pass_state: i64,
    high_pass_state: i64,
}

impl SidExternalFilter {
    /// # Errors
    ///
    /// Returns [`SidTimingError`] when `processor_clock_hz` is zero.
    pub fn new(processor_clock_hz: u32) -> Result<Self, SidTimingError> {
        if processor_clock_hz == 0 {
            return Err(SidTimingError);
        }

        Ok(Self::new_with_valid_clock(processor_clock_hz))
    }

    pub(crate) fn new_with_valid_clock(processor_clock_hz: u32) -> Self {
        debug_assert_ne!(processor_clock_hz, 0);

        // RC coefficients are rational after substituting the board component
        // values. Quantize them once without introducing host floating point.
        let clock = i64::from(processor_clock_hz);
        let low_pass_coefficient = round_positive_ratio(128 * 100_000, 100_000 + clock);
        let high_pass_coefficient = round_positive_ratio(131_072 * 100, 100 + clock);
        Self {
            low_pass_coefficient,
            high_pass_coefficient,
            low_pass_state: 0,
            high_pass_state: 0,
        }
    }

    pub const fn output_pcm(&self) -> i64 {
        (self.low_pass_state - self.high_pass_state) >> STATE_FRACTION_BITS
    }

    pub fn reset(&mut self) {
        self.low_pass_state = 0;
        self.high_pass_state = 0;
    }

    /// # Errors
    ///
    /// Returns [`SidPcmRangeError`] when `input_pcm` is outside signed 16-bit PCM.
    pub fn clock(&mut self, input_pcm: i32) -> Result<i64, SidPcmRangeError> {
        let input = i16::try_from(input_pcm).map_err(|_| SidPcmRangeError { value: input_pcm })?;
        Ok(self.clock_pcm(input))
    }

    /// Hot path for an input already clamped by the internal SID filter.
    pub fn clock_pcm(&mut self, input_pcm: i16) -> i64 {
        let scaled_input = i64::from(input_pcm) * STATE_SCALE;
        let low_pass_delta = (self.low_pass_coefficient * (scaled_input - self.low_pass_state))
            >> LOW_PASS_COEFFICIENT_BITS;
        let high_pass_delta = (self.high_pass_coefficient
            * (self.low_pass_state - self.high_pass_state))
            >> HIGH_PASS_COEFFICIENT_BITS;
        self.low_pass_state += low_pass_delta;
        self.high_pass_state += high_pass_delta;
        self.output_pcm()
    }
}

const fn round_positive_ratio(numerator: i64, denominator: i64) -> i64 {
    (numerator * 2 + denominator) / (denominator * 2)
}

#[cfg(test)]
mod tests {
    use super::SidExternalFilter;

    #[test]
    fn reproduces_resid_one_megahertz_coefficients() {
        let mut filter = SidExternalFilter::new(1_000_000).expect("valid clock");

        assert_eq!(filter.clock(0x4000), Ok(1_536));
        assert_eq!(filter.clock(0x4000), Ok(2_927));
    }

    #[test]
    fn board_high_pass_removes_sustained_dc() {
        let mut filter = SidExternalFilter::new(1_000_000).expect("valid clock");
        let mut output = 0;
        for _ in 0..150_000 {
            output = filter.clock_pcm(12_000);
        }

        assert!(output.abs() < 10);
    }

    #[test]
    fn rejects_out_of_range_samples() {
        let mut filter = SidExternalFilter::new(985_248).expect("valid clock");

        assert!(filter.clock(0x8000).is_err());
        assert!(filter.clock(-0x8001).is_err());
    }
}
