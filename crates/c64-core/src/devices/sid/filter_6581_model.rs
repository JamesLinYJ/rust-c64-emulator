// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - nonlinear MOS 6581 filter circuit model
//
//   File:       sid/filter_6581_model.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

// These conversions are the explicit quantization boundaries of the measured
// reSID analog model. Every destination range is fixed by the table format.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use core::fmt;
use std::sync::OnceLock;

const NORMALIZED_VOLTAGE_LEVEL_COUNT: usize = 1 << 16;
const NORMALIZED_VOLTAGE_MAXIMUM: i32 = (NORMALIZED_VOLTAGE_LEVEL_COUNT - 1) as i32;
const NORMALIZED_VOLTAGE_MIDPOINT: i32 = 1 << 15;
const NORMALIZED_CAPACITOR_FRACTION_BITS: u32 = 15;
const FILTER_CUTOFF_LEVEL_COUNT: usize = 1 << 11;
const FILTER_GAIN_LEVEL_COUNT: usize = 1 << 4;
const FILTER_SUMMER_CONFIGURATION_COUNT: usize = 5;
const AUDIO_MIXER_CONFIGURATION_COUNT: usize = 8;
const SID_VOICE_DIGITAL_FRACTION_BITS: u32 = 18;
const SID_EXTERNAL_INPUT_FRACTION_BITS: u32 = 14;
const SID_EXTERNAL_INPUT_VOICE_SPAN: i64 = 3;
const SID_FILTER_REFERENCE_CLOCK_HZ: f64 = 1_000_000.0;

const MOS6581_OP_AMP_POINTS: [[f64; 2]; 35] = [
    [0.81, 10.31],
    [0.81, 10.31],
    [2.4, 10.31],
    [2.6, 10.3],
    [2.7, 10.29],
    [2.8, 10.26],
    [2.9, 10.17],
    [3.0, 10.04],
    [3.1, 9.83],
    [3.2, 9.58],
    [3.3, 9.32],
    [3.5, 8.69],
    [3.7, 8.0],
    [4.0, 6.89],
    [4.4, 5.21],
    [4.54, 4.54],
    [4.6, 4.19],
    [4.8, 3.0],
    [4.9, 2.3],
    [4.95, 2.03],
    [5.0, 1.88],
    [5.05, 1.77],
    [5.1, 1.69],
    [5.2, 1.58],
    [5.4, 1.44],
    [5.6, 1.33],
    [5.8, 1.26],
    [6.0, 1.21],
    [6.4, 1.12],
    [7.0, 1.02],
    [7.5, 0.97],
    [8.5, 0.89],
    [10.0, 0.81],
    [10.31, 0.81],
    [10.31, 0.81],
];

const MOS6581_VOICE_VOLTAGE_RANGE: f64 = 1.5;
const MOS6581_VOICE_DC_VOLTS: f64 = 5.0;
const MOS6581_CAPACITANCE_FARADS: f64 = 470e-12;
const MOS6581_SUPPLY_VOLTS: f64 = 12.18;
const MOS6581_THRESHOLD_VOLTS: f64 = 1.31;
const MOS6581_THERMAL_VOLTS: f64 = 26e-3;
const MOS6581_GATE_COUPLING: f64 = 1.0;
const MOS6581_TRANSISTOR_UCOX: f64 = 20e-6;
const MOS6581_VCR_WIDTH_LENGTH_RATIO: f64 = 9.0;
const MOS6581_SNAKE_WIDTH_LENGTH_RATIO: f64 = 1.0 / 115.0;
const MOS6581_CUTOFF_DAC_ZERO_VOLTS: f64 = 6.65;
const MOS6581_CUTOFF_DAC_SCALE_VOLTS: f64 = 2.63;
const MOS6581_CUTOFF_DAC_RESISTOR_RATIO: f64 = 2.2;
const MOS6581_DAC_LEAKAGE: f64 = 0.0075;

#[derive(Clone, Copy)]
struct SplinePoint {
    x: usize,
    y: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Sid6581IntegratorState {
    capacitor_voltage: i32,
    op_amp_input: u16,
}

struct OpAmpTransferTable {
    derivative: Vec<i16>,
    input: Vec<u16>,
    lower_root_bound: i32,
    upper_root_bound: i32,
}

/// Immutable measured transfer tables shared by all MOS 6581 instances.
pub(crate) struct Sid6581FilterModel {
    cutoff_dac: Vec<u16>,
    gain: Vec<u16>,
    mixer: Vec<u16>,
    op_amp_reverse: Vec<u16>,
    summer: Vec<u16>,
    vcr_gate_voltage: Vec<u16>,
    vcr_current_term: Vec<u16>,
    voice_dc: i32,
    voice_scale: i32,
    normalized_supply_threshold: i32,
    normalized_snake_current: i32,
}

impl fmt::Debug for Sid6581FilterModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Sid6581FilterModel")
            .field("cutoff_entries", &self.cutoff_dac.len())
            .field("gain_entries", &self.gain.len())
            .field("mixer_entries", &self.mixer.len())
            .field("summer_entries", &self.summer.len())
            .finish_non_exhaustive()
    }
}

impl Sid6581FilterModel {
    fn new() -> Self {
        let minimum_voltage = MOS6581_OP_AMP_POINTS[0][0];
        let maximum_op_amp_voltage = MOS6581_OP_AMP_POINTS[0][1];
        let coupled_supply_threshold =
            MOS6581_GATE_COUPLING * (MOS6581_SUPPLY_VOLTS - MOS6581_THRESHOLD_VOLTS);
        let maximum_voltage = coupled_supply_threshold.max(maximum_op_amp_voltage);
        let voltage_span = maximum_voltage - minimum_voltage;
        let normalized_16 = f64::from(NORMALIZED_VOLTAGE_MAXIMUM) / voltage_span;
        let normalized_16_integer = normalized_16.trunc() as i32;
        let normalized_31 = f64::from(i32::MAX) / voltage_span;

        let voice_scale =
            ((f64::from(1 << 14) / voltage_span) * MOS6581_VOICE_VOLTAGE_RANGE).trunc() as i32;
        let voice_dc = (normalized_16 * (MOS6581_VOICE_DC_VOLTS - minimum_voltage)).trunc() as i32;
        let normalized_supply_threshold =
            (normalized_16 * (coupled_supply_threshold - minimum_voltage) + 0.5).trunc() as i32;
        let normalized_snake_current = (voltage_span
            * f64::from(1 << 13)
            * ((MOS6581_TRANSISTOR_UCOX
                / (2.0 * MOS6581_GATE_COUPLING)
                / MOS6581_CAPACITANCE_FARADS)
                * MOS6581_SNAKE_WIDTH_LENGTH_RATIO
                * (1.0 / SID_FILTER_REFERENCE_CLOCK_HZ))
            + 0.5)
            .trunc() as i32;

        let op_amp = build_op_amp_transfer_table(normalized_16, normalized_31, minimum_voltage);
        let gain = build_gain_table(&op_amp, normalized_supply_threshold);
        let summer = build_summer_table(&op_amp, normalized_supply_threshold);
        let mixer = build_mixer_table(&op_amp, normalized_supply_threshold);
        let op_amp_reverse = op_amp.input.clone();
        let cutoff_dac = build_cutoff_dac(normalized_16, minimum_voltage);
        let vcr_gate_voltage = build_vcr_gate_voltage_table(normalized_16_integer, minimum_voltage);
        let vcr_current_term = build_vcr_current_table(normalized_16_integer);

        Self {
            cutoff_dac,
            gain,
            mixer,
            op_amp_reverse,
            summer,
            vcr_gate_voltage,
            vcr_current_term,
            voice_dc,
            voice_scale,
            normalized_supply_threshold,
            normalized_snake_current,
        }
    }

    pub(crate) const fn create_integrator_state() -> Sid6581IntegratorState {
        Sid6581IntegratorState {
            capacitor_voltage: 0,
            op_amp_input: 0,
        }
    }

    pub(crate) fn reset_integrator_state(state: &mut Sid6581IntegratorState) {
        state.capacitor_voltage = 0;
        state.op_amp_input = 0;
    }

    pub(crate) fn scale_voice(&self, sample: i32) -> i32 {
        let scaled = i64::from(sample) * i64::from(self.voice_scale);
        (scaled >> SID_VOICE_DIGITAL_FRACTION_BITS) as i32 + self.voice_dc
    }

    pub(crate) fn scale_external_input(&self, sample: i32) -> i32 {
        let scaled =
            i64::from(sample) * i64::from(self.voice_scale) * SID_EXTERNAL_INPUT_VOICE_SPAN;
        (scaled >> SID_EXTERNAL_INPUT_FRACTION_BITS) as i32 + i32::from(self.mixer[0])
    }

    pub(crate) fn cutoff_control_voltage_squared(&self, cutoff: u16) -> i64 {
        let control_voltage = i32::from(self.cutoff_dac[usize::from(cutoff & 0x07ff)]);
        let difference = self.normalized_supply_threshold - control_voltage;
        i64::from(difference) * i64::from(difference) / 2
    }

    pub(crate) fn resonance_gain(&self, resonance: u8, band_pass_voltage: i32) -> i32 {
        let gain = usize::from(!resonance & 0x0f);
        let index = gain * NORMALIZED_VOLTAGE_LEVEL_COUNT + band_pass_voltage as usize;
        i32::from(self.gain[index])
    }

    pub(crate) fn sum_filter_inputs(&self, routed_input_count: usize, voltage_sum: i32) -> i32 {
        let offset =
            routed_input_count * (routed_input_count + 3) / 2 * NORMALIZED_VOLTAGE_LEVEL_COUNT;
        i32::from(self.summer[offset + voltage_sum as usize])
    }

    pub(crate) fn mix_audio_inputs(&self, input_count: usize, voltage_sum: i32) -> i32 {
        let offset = if input_count == 0 {
            0
        } else {
            1 + (input_count - 1) * input_count * NORMALIZED_VOLTAGE_LEVEL_COUNT / 2
        };
        i32::from(self.mixer[offset + voltage_sum as usize])
    }

    pub(crate) fn apply_volume(&self, volume: u8, mixed_voltage: i32) -> i16 {
        let index =
            usize::from(volume & 0x0f) * NORMALIZED_VOLTAGE_LEVEL_COUNT + mixed_voltage as usize;
        let output = i32::from(self.gain[index]) - NORMALIZED_VOLTAGE_MIDPOINT;
        output as i16
    }

    pub(crate) fn integrate(
        &self,
        input_voltage: i32,
        state: &mut Sid6581IntegratorState,
        cutoff_voltage_squared: i64,
    ) -> i32 {
        let supply_threshold = self.normalized_supply_threshold;
        let gate_source = supply_threshold - i32::from(state.op_amp_input);
        let gate_drain = supply_threshold - input_voltage;
        let gate_drain_squared = gate_drain.wrapping_mul(gate_drain) as u32;
        let snake_difference =
            (gate_source.wrapping_mul(gate_source) as u32).wrapping_sub(gate_drain_squared) as i32;
        let snake_current = self
            .normalized_snake_current
            .wrapping_mul(snake_difference >> 15);
        let gate_lookup_index = ((cutoff_voltage_squared + i64::from(gate_drain_squared >> 1))
            / NORMALIZED_VOLTAGE_LEVEL_COUNT as i64) as usize;
        let gate_voltage = i32::from(self.vcr_gate_voltage[gate_lookup_index]);
        let gate_source_voltage = (gate_voltage - i32::from(state.op_amp_input)).max(0);
        let gate_drain_voltage = (gate_voltage - input_voltage).max(0);
        let source_current = i32::from(self.vcr_current_term[gate_source_voltage as usize]);
        let drain_current = i32::from(self.vcr_current_term[gate_drain_voltage as usize]);
        let vcr_current = (source_current - drain_current).wrapping_mul(1 << 15);

        state.capacitor_voltage = state
            .capacitor_voltage
            .wrapping_sub(snake_current)
            .wrapping_sub(vcr_current);
        let reverse_index = (state.capacitor_voltage >> NORMALIZED_CAPACITOR_FRACTION_BITS)
            + NORMALIZED_VOLTAGE_MIDPOINT;
        state.op_amp_input = self.op_amp_reverse[reverse_index as usize];
        i32::from(state.op_amp_input) + (state.capacitor_voltage >> 14)
    }
}

static SHARED_MODEL: OnceLock<Sid6581FilterModel> = OnceLock::new();

pub(crate) fn shared_mos6581_filter_model() -> &'static Sid6581FilterModel {
    SHARED_MODEL.get_or_init(Sid6581FilterModel::new)
}

#[allow(clippy::manual_midpoint)]
fn build_op_amp_transfer_table(
    normalized_16: f64,
    normalized_31: f64,
    minimum_voltage: f64,
) -> OpAmpTransferTable {
    let mut scaled_points = vec![SplinePoint { x: 0, y: 0.0 }; MOS6581_OP_AMP_POINTS.len()];
    for (source_index, point) in MOS6581_OP_AMP_POINTS.iter().enumerate() {
        scaled_points[MOS6581_OP_AMP_POINTS.len() - 1 - source_index] = SplinePoint {
            x: ((normalized_16 * (point[1] - point[0]) + NORMALIZED_VOLTAGE_LEVEL_COUNT as f64)
                / 2.0
                + 0.5)
                .trunc() as usize,
            y: normalized_31 * (point[0] - minimum_voltage),
        };
    }
    let last_index = scaled_points.len() - 1;
    if scaled_points[last_index].x >= NORMALIZED_VOLTAGE_LEVEL_COUNT {
        scaled_points[last_index].x = NORMALIZED_VOLTAGE_LEVEL_COUNT - 1;
        scaled_points[last_index - 1].x = NORMALIZED_VOLTAGE_LEVEL_COUNT - 1;
    }

    let interpolated = interpolate_spline(&scaled_points);
    let mut input = vec![0_u16; NORMALIZED_VOLTAGE_LEVEL_COUNT];
    let mut derivative = vec![0_i16; NORMALIZED_VOLTAGE_LEVEL_COUNT];
    let lower_root_bound = scaled_points[0].x;
    let upper_root_bound = scaled_points[last_index].x;
    let mut previous = interpolated[lower_root_bound];
    for index in lower_root_bound..=upper_root_bound {
        let current = interpolated[index];
        let delta = i64::from(current) - i64::from(previous);
        input[index] =
            if u64::from(current) > u64::from(NORMALIZED_VOLTAGE_MAXIMUM as u32) * (1 << 15) {
                u16::MAX
            } else {
                (current >> 15) as u16
            };
        derivative[index] = (delta >> 4) as i16;
        previous = current;
    }
    OpAmpTransferTable {
        derivative,
        input,
        lower_root_bound: lower_root_bound as i32,
        upper_root_bound: upper_root_bound as i32,
    }
}

fn interpolate_spline(points: &[SplinePoint]) -> Vec<u32> {
    let mut output = vec![0_u32; NORMALIZED_VOLTAGE_LEVEL_COUNT];
    for segment in 0..points.len() - 3 {
        let point_0 = points[segment];
        let point_1 = points[segment + 1];
        let point_2 = points[segment + 2];
        let point_3 = points[segment + 3];
        if point_1.x == point_2.x {
            continue;
        }

        let (slope_1, slope_2) = if point_0.x == point_1.x && point_2.x == point_3.x {
            let slope = (point_2.y - point_1.y) / (point_2.x - point_1.x) as f64;
            (slope, slope)
        } else if point_0.x == point_1.x {
            let slope_2 = (point_3.y - point_1.y) / (point_3.x - point_1.x) as f64;
            let slope_1 =
                (3.0 * ((point_2.y - point_1.y) / (point_2.x - point_1.x) as f64) - slope_2) / 2.0;
            (slope_1, slope_2)
        } else if point_2.x == point_3.x {
            let slope_1 = (point_2.y - point_0.y) / (point_2.x - point_0.x) as f64;
            let slope_2 =
                (3.0 * ((point_2.y - point_1.y) / (point_2.x - point_1.x) as f64) - slope_1) / 2.0;
            (slope_1, slope_2)
        } else {
            (
                (point_2.y - point_0.y) / (point_2.x - point_0.x) as f64,
                (point_3.y - point_1.y) / (point_3.x - point_1.x) as f64,
            )
        };
        interpolate_spline_segment(&mut output, point_1, point_2, slope_1, slope_2);
    }
    output
}

fn interpolate_spline_segment(
    output: &mut [u32],
    start: SplinePoint,
    end: SplinePoint,
    start_slope: f64,
    end_slope: f64,
) {
    let start_x = start.x as f64;
    let end_x = end.x as f64;
    let width = end_x - start_x;
    let height = end.y - start.y;
    let cubic = (start_slope + end_slope - 2.0 * height / width) / (width * width);
    let quadratic = ((end_slope - start_slope) / width - 3.0 * (start_x + end_x) * cubic) / 2.0;
    let linear = start_slope - (3.0 * start_x * cubic + 2.0 * quadratic) * start_x;
    let constant = start.y - ((start_x * cubic + quadratic) * start_x + linear) * start_x;
    let mut value = ((cubic * start_x + quadratic) * start_x + linear) * start_x + constant;
    let mut delta =
        (3.0 * cubic * (start_x + 1.0) + 2.0 * quadratic) * start_x + cubic + quadratic + linear;
    let mut second_delta = 6.0 * cubic * (start_x + 1.0) + 2.0 * quadratic;
    let third_delta = 6.0 * cubic;
    for cell in &mut output[start.x..=end.x] {
        *cell = (value.max(0.0) + 0.5).trunc() as u32;
        value += delta;
        delta += second_delta;
        second_delta += third_delta;
    }
}

fn build_gain_table(op_amp: &OpAmpTransferTable, supply_threshold: i32) -> Vec<u16> {
    let mut table = vec![0_u16; FILTER_GAIN_LEVEL_COUNT * NORMALIZED_VOLTAGE_LEVEL_COUNT];
    for gain in 0..FILTER_GAIN_LEVEL_COUNT {
        let mut root = op_amp.lower_root_bound;
        let loading = (gain << 4) as i32;
        let offset = gain * NORMALIZED_VOLTAGE_LEVEL_COUNT;
        for voltage in 0..NORMALIZED_VOLTAGE_LEVEL_COUNT {
            let solved = solve_gain(op_amp, supply_threshold, loading, voltage as i32, root);
            root = solved.0;
            table[offset + voltage] = solved.1;
        }
    }
    table
}

fn build_summer_table(op_amp: &OpAmpTransferTable, supply_threshold: i32) -> Vec<u16> {
    let mut table = vec![0_u16; summer_offset(FILTER_SUMMER_CONFIGURATION_COUNT)];
    let mut offset = 0;
    for configuration in 0..FILTER_SUMMER_CONFIGURATION_COUNT {
        let input_count = configuration + 2;
        let size = input_count * NORMALIZED_VOLTAGE_LEVEL_COUNT;
        let mut root = op_amp.lower_root_bound;
        for voltage_sum in 0..size {
            let solved = solve_gain(
                op_amp,
                supply_threshold,
                (input_count << 7) as i32,
                (voltage_sum / input_count) as i32,
                root,
            );
            root = solved.0;
            table[offset + voltage_sum] = solved.1;
        }
        offset += size;
    }
    table
}

fn build_mixer_table(op_amp: &OpAmpTransferTable, supply_threshold: i32) -> Vec<u16> {
    let mut table = vec![0_u16; mixer_offset(AUDIO_MIXER_CONFIGURATION_COUNT)];
    let mut offset = 0;
    let mut size = 1;
    for configuration in 0..AUDIO_MIXER_CONFIGURATION_COUNT {
        let physical_input_count = configuration;
        let divisor = physical_input_count.max(1);
        let loading = (((physical_input_count << 7) * 8) / 6) as i32;
        let mut root = op_amp.lower_root_bound;
        for voltage_sum in 0..size {
            let solved = solve_gain(
                op_amp,
                supply_threshold,
                loading,
                (voltage_sum / divisor) as i32,
                root,
            );
            root = solved.0;
            table[offset + voltage_sum] = solved.1;
        }
        offset += size;
        size = (configuration + 1) * NORMALIZED_VOLTAGE_LEVEL_COUNT;
    }
    table
}

fn solve_gain(
    op_amp: &OpAmpTransferTable,
    supply_threshold: i32,
    loading: i32,
    input_voltage: i32,
    initial_root: i32,
) -> (i32, u16) {
    let mut lower = op_amp.lower_root_bound;
    let mut upper = op_amp.upper_root_bound;
    let mut root = initial_root;
    let scaled_loading = loading + (1 << 7);
    let supply_minus_input = (supply_threshold - input_voltage).max(0);
    let input_current = i64::from(loading)
        * ((i64::from(supply_minus_input) * i64::from(supply_minus_input)) >> 12);

    loop {
        let previous_root = root;
        let input = i32::from(op_amp.input[root as usize]);
        let derivative = i32::from(op_amp.derivative[root as usize]);
        let mut output = input + root * 2 - NORMALIZED_VOLTAGE_LEVEL_COUNT as i32;
        output = output.clamp(0, NORMALIZED_VOLTAGE_MAXIMUM);
        let supply_minus_op_amp = (supply_threshold - input).max(0);
        let supply_minus_output = (supply_threshold - output).max(0);
        let value = i64::from(scaled_loading)
            * ((i64::from(supply_minus_op_amp) * i64::from(supply_minus_op_amp)) >> 12)
            - input_current
            - ((i64::from(supply_minus_output) * i64::from(supply_minus_output)) >> 5);
        let divisor_first =
            (i64::from(supply_minus_output) * i64::from(derivative + (1 << 11))) >> 1;
        let divisor_second = i64::from(scaled_loading)
            * ((i64::from(supply_minus_op_amp) * i64::from(derivative)) >> 8);
        let divisor = (divisor_first - divisor_second) >> 14;
        if divisor != 0 {
            root -= (value / divisor) as i32;
        }
        if root == previous_root {
            return (root, output as u16);
        }

        if value < 0 {
            lower = previous_root;
        } else {
            upper = previous_root;
        }
        if root <= lower || root >= upper {
            root = (lower + upper) >> 1;
            if root == lower {
                return (root, output as u16);
            }
        }
    }
}

fn build_cutoff_dac(normalized_16: f64, minimum_voltage: f64) -> Vec<u16> {
    let raw_dac = build_dac_table(11, MOS6581_CUTOFF_DAC_RESISTOR_RATIO, false);
    let mut output = vec![0_u16; FILTER_CUTOFF_LEVEL_COUNT];
    for (value, cell) in output.iter_mut().enumerate() {
        *cell = (normalized_16
            * (MOS6581_CUTOFF_DAC_ZERO_VOLTS
                + f64::from(raw_dac[value]) * MOS6581_CUTOFF_DAC_SCALE_VOLTS
                    / FILTER_CUTOFF_LEVEL_COUNT as f64
                - minimum_voltage)
            + 0.5)
            .trunc() as u16;
    }
    output
}

fn build_dac_table(bits: usize, resistor_ratio: f64, terminated: bool) -> Vec<u16> {
    let mut bit_voltages = vec![0.0; bits];
    for (set_bit, bit_voltage) in bit_voltages.iter_mut().enumerate() {
        let mut voltage = 1.0;
        let resistor = 1.0;
        let double_resistor = resistor_ratio * resistor;
        let mut tail_resistance = if terminated {
            double_resistor
        } else {
            f64::INFINITY
        };
        let mut bit = 0;
        while bit < set_bit {
            tail_resistance = if tail_resistance.is_finite() {
                resistor + double_resistor * tail_resistance / (double_resistor + tail_resistance)
            } else {
                resistor + double_resistor
            };
            bit += 1;
        }
        if tail_resistance.is_finite() {
            tail_resistance =
                double_resistor * tail_resistance / (double_resistor + tail_resistance);
            voltage = voltage * tail_resistance / double_resistor;
        } else {
            tail_resistance = double_resistor;
        }
        bit += 1;
        while bit < bits {
            tail_resistance += resistor;
            let current = voltage / tail_resistance;
            tail_resistance =
                double_resistor * tail_resistance / (double_resistor + tail_resistance);
            voltage = tail_resistance * current;
            bit += 1;
        }
        *bit_voltage = voltage;
    }

    let mut output = vec![0_u16; 1 << bits];
    for (value, cell) in output.iter_mut().enumerate() {
        let mut remaining = value;
        let mut voltage = 0.0;
        for bit_voltage in &bit_voltages {
            voltage += if remaining & 1 != 0 {
                *bit_voltage
            } else {
                MOS6581_DAC_LEAKAGE * bit_voltage
            };
            remaining >>= 1;
        }
        *cell = (f64::from((1_u32 << bits) - 1) * voltage + 0.5).trunc() as u16;
    }
    output
}

fn build_vcr_gate_voltage_table(normalized_16: i32, minimum_voltage: f64) -> Vec<u16> {
    let mut output = vec![0_u16; NORMALIZED_VOLTAGE_LEVEL_COUNT];
    let normalized_minimum = f64::from(normalized_16) * minimum_voltage;
    let normalized_threshold = f64::from(normalized_16)
        * MOS6581_GATE_COUPLING
        * (MOS6581_SUPPLY_VOLTS - MOS6581_THRESHOLD_VOLTS);
    for (index, cell) in output.iter_mut().enumerate() {
        let gate_voltage =
            normalized_threshold - (index as f64 * NORMALIZED_VOLTAGE_LEVEL_COUNT as f64).sqrt();
        *cell = (MOS6581_GATE_COUPLING * gate_voltage - normalized_minimum + 0.5).trunc() as u16;
    }
    output
}

fn build_vcr_current_table(normalized_16: i32) -> Vec<u16> {
    let mut output = vec![0_u16; NORMALIZED_VOLTAGE_LEVEL_COUNT];
    let coupled_threshold = MOS6581_GATE_COUPLING * MOS6581_THRESHOLD_VOLTS;
    let saturation_current = (2.0 * MOS6581_TRANSISTOR_UCOX * MOS6581_THERMAL_VOLTS.powi(2)
        / MOS6581_GATE_COUPLING)
        * MOS6581_VCR_WIDTH_LENGTH_RATIO;
    let normalized_current = (f64::from(normalized_16)
        / 2.0
        / SID_FILTER_REFERENCE_CLOCK_HZ
        / MOS6581_CAPACITANCE_FARADS)
        * saturation_current;
    for (voltage, cell) in output.iter_mut().enumerate() {
        let logarithm = ((voltage as f64 / f64::from(normalized_16) - coupled_threshold)
            / (2.0 * MOS6581_THERMAL_VOLTS))
            .exp()
            .ln_1p();
        *cell = (normalized_current * logarithm * logarithm).trunc() as u16;
    }
    output
}

const fn summer_offset(input_count: usize) -> usize {
    input_count * (input_count + 3) / 2 * NORMALIZED_VOLTAGE_LEVEL_COUNT
}

const fn mixer_offset(input_count: usize) -> usize {
    if input_count == 0 {
        0
    } else {
        1 + (input_count - 1) * input_count * NORMALIZED_VOLTAGE_LEVEL_COUNT / 2
    }
}
