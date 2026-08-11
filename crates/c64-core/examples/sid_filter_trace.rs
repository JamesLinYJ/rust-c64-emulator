// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic SID filter differential adapter
//
//   File:       sid_filter_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::{SidFilter, SidModel};
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
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Clock {
        voice_1: i32,
        voice_2: i32,
        voice_3: i32,
        external_input: Option<i32>,
    },
    Registers {
        cutoff: u16,
        resonance_routing: u8,
        mode_volume: u8,
    },
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    cutoff: u16,
    output_pcm: i16,
}

fn observe(filter: &SidFilter) -> Observation {
    Observation {
        cutoff: filter.cutoff(),
        output_pcm: filter.output_pcm(),
    }
}

fn run_scenario(scenario: Scenario) -> Vec<Observation> {
    let mut filter = SidFilter::new(scenario.model.into());
    scenario
        .operations
        .into_iter()
        .map(|operation| {
            match operation {
                Operation::Clock {
                    voice_1,
                    voice_2,
                    voice_3,
                    external_input,
                } => {
                    filter.clock(voice_1, voice_2, voice_3, external_input);
                }
                Operation::Registers {
                    cutoff,
                    resonance_routing,
                    mode_volume,
                } => {
                    filter.set_cutoff(cutoff);
                    filter.set_resonance_routing(resonance_routing);
                    filter.set_mode_volume(mode_volume);
                }
                Operation::Reset => filter.reset(),
            }
            observe(&filter)
        })
        .collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let scenarios: Vec<Scenario> = serde_json::from_str(&input)?;
    let output: Vec<Vec<Observation>> = scenarios.into_iter().map(run_scenario).collect();
    serde_json::to_writer(io::stdout().lock(), &output)?;
    Ok(())
}
