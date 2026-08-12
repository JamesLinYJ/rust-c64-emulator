// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - complete SID device differential adapter
//
//   File:       sid_device_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io;

use c64_core::{Sid, SidModel, SidVoiceState};
use serde::{Deserialize, Serialize};

#[path = "support/json_line.rs"]
mod json_line;

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
    processor_clock_hz: u32,
    sample_rate_hz: u32,
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Clock { cycles: u32 },
    Drain { maximum_length: Option<usize> },
    Paddles { x: u8, y: u8 },
    Read { address: u16 },
    Reset,
    Write { address: u16, value: u8 },
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceObservation {
    attack_decay: u8,
    control: u8,
    envelope: u8,
    frequency: u16,
    pulse_width: u16,
    sustain_release: u8,
}

impl From<SidVoiceState> for VoiceObservation {
    fn from(state: SidVoiceState) -> Self {
        Self {
            attack_decay: state.attack_decay,
            control: state.control,
            envelope: state.envelope,
            frequency: state.frequency,
            pulse_width: state.pulse_width,
            sustain_release: state.sustain_release,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    filter_cutoff: u16,
    master_volume: u8,
    pending_sample_count: usize,
    read_value: Option<u8>,
    sample_bits: Vec<u32>,
    voices: [VoiceObservation; 3],
}

fn observe(sid: &Sid, read_value: Option<u8>, sample_bits: Vec<u32>) -> Observation {
    let voices = std::array::from_fn(|index| {
        sid.voice_state(index).map_or(
            VoiceObservation {
                attack_decay: 0,
                control: 0,
                envelope: 0,
                frequency: 0,
                pulse_width: 0,
                sustain_release: 0,
            },
            VoiceObservation::from,
        )
    });
    Observation {
        filter_cutoff: sid.filter_cutoff(),
        master_volume: sid.master_volume(),
        pending_sample_count: sid.pending_sample_count(),
        read_value,
        sample_bits,
        voices,
    }
}

fn run_scenario(scenario: Scenario) -> Result<Vec<Observation>, io::Error> {
    let mut sid = Sid::new(
        scenario.model.into(),
        scenario.processor_clock_hz,
        scenario.sample_rate_hz,
    )
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let mut observations = Vec::with_capacity(scenario.operations.len());
    for operation in scenario.operations {
        let mut read_value = None;
        let mut sample_bits = Vec::new();
        match operation {
            Operation::Clock { cycles } => sid.clock_cycles(cycles),
            Operation::Drain { maximum_length } => {
                sample_bits = sid
                    .drain_samples(maximum_length)
                    .into_iter()
                    .map(f32::to_bits)
                    .collect();
            }
            Operation::Paddles { x, y } => sid.set_paddle_inputs(x, y),
            Operation::Read { address } => read_value = Some(sid.read(address)),
            Operation::Reset => sid.reset(),
            Operation::Write { address, value } => sid.write(address, value),
        }
        observations.push(observe(&sid, read_value, sample_bits));
    }
    Ok(observations)
}

fn main() -> Result<(), Box<dyn Error>> {
    let scenarios: Vec<Scenario> = json_line::read_stdin_json_line()?;
    let output = scenarios
        .into_iter()
        .map(run_scenario)
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_writer(io::stdout().lock(), &output)?;
    Ok(())
}
