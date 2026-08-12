// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic SID oscillator differential adapter
//
//   File:       sid_oscillator_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::{SidModel, SidOscillator};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize)]
enum Model {
    #[serde(rename = "6581")]
    Mos6581,
    #[serde(rename = "8580")]
    Mos8580,
}

impl From<Model> for SidModel {
    fn from(model: Model) -> Self {
        match model {
            Model::Mos6581 => Self::Mos6581,
            Model::Mos8580 => Self::Mos8580,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    model: Model,
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Operation {
    ClockCycle,
    Control { index: usize, value: u8 },
    Frequency { index: usize, value: u16 },
    PulseWidth { index: usize, value: u16 },
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    readback: [u8; 3],
    waveform: [u16; 3],
}

fn require_index(index: usize) -> Result<usize, io::Error> {
    if index < 3 {
        Ok(index)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("SID oscillator index {index} is outside 0..3"),
        ))
    }
}

fn source_index(index: usize) -> usize {
    match index {
        0 => 2,
        1 => 0,
        _ => 1,
    }
}

fn clock_ring(oscillators: &mut [SidOscillator; 3]) {
    for oscillator in oscillators.iter_mut() {
        oscillator.clock_cycle();
    }
    let rising: [bool; 3] = std::array::from_fn(|index| oscillators[index].msb_rising());
    let sync: [bool; 3] = std::array::from_fn(|index| oscillators[index].sync_enabled());
    let reset = [
        rising[2] && sync[0] && !(sync[2] && rising[1]),
        rising[0] && sync[1] && !(sync[0] && rising[2]),
        rising[1] && sync[2] && !(sync[1] && rising[0]),
    ];
    for (oscillator, reset_accumulator) in oscillators.iter_mut().zip(reset) {
        if reset_accumulator {
            oscillator.reset_accumulator_for_sync();
        }
    }
    let accumulators: [u32; 3] = std::array::from_fn(|index| oscillators[index].accumulator());
    for index in 0..3 {
        oscillators[index]
            .update_waveform_output_with_sync_source(accumulators[source_index(index)]);
    }
}

fn observe(oscillators: &[SidOscillator; 3]) -> Observation {
    Observation {
        readback: std::array::from_fn(|index| oscillators[index].oscillator_readback()),
        waveform: std::array::from_fn(|index| oscillators[index].waveform_output()),
    }
}

fn run_scenario(scenario: Scenario) -> Result<Vec<Observation>, io::Error> {
    let model = SidModel::from(scenario.model);
    let mut oscillators = [
        SidOscillator::new(model),
        SidOscillator::new(model),
        SidOscillator::new(model),
    ];
    let mut observations = Vec::with_capacity(scenario.operations.len());
    for operation in scenario.operations {
        match operation {
            Operation::ClockCycle => clock_ring(&mut oscillators),
            Operation::Control { index, value } => {
                let index = require_index(index)?;
                let source_accumulator = oscillators[source_index(index)].accumulator();
                oscillators[index].set_control_with_sync_source(value, source_accumulator);
            }
            Operation::Frequency { index, value } => {
                oscillators[require_index(index)?].set_frequency(value);
            }
            Operation::PulseWidth { index, value } => {
                oscillators[require_index(index)?].set_pulse_width(value);
            }
            Operation::Reset => {
                for oscillator in &mut oscillators {
                    oscillator.reset();
                }
            }
        }
        observations.push(observe(&oscillators));
    }
    Ok(observations)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let scenarios: Vec<Scenario> = serde_json::from_str(&input)?;
    let output = scenarios
        .into_iter()
        .map(run_scenario)
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_writer(io::stdout().lock(), &output)?;
    Ok(())
}
