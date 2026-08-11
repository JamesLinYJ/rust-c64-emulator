// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic SID envelope differential adapter
//
//   File:       sid_envelope_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::SidEnvelopeGenerator;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Operation {
    AttackDecay { value: u8 },
    SustainRelease { value: u8 },
    Control { value: u8 },
    ClockCycle,
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    output: u8,
    readback: u8,
}

fn observe(envelope: &SidEnvelopeGenerator) -> Observation {
    Observation {
        output: envelope.output(),
        readback: envelope.readback(),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let operations: Vec<Operation> = serde_json::from_str(&input)?;
    let mut envelope = SidEnvelopeGenerator::new();
    let mut observations = Vec::with_capacity(operations.len());
    for operation in operations {
        match operation {
            Operation::AttackDecay { value } => envelope.write_attack_decay(value),
            Operation::SustainRelease { value } => envelope.write_sustain_release(value),
            Operation::Control { value } => envelope.write_control(value),
            Operation::ClockCycle => envelope.clock_cycle(),
            Operation::Reset => envelope.reset(),
        }
        observations.push(observe(&envelope));
    }
    serde_json::to_writer(io::stdout().lock(), &observations)?;
    Ok(())
}
