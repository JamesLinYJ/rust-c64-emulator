// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic 1541 GCR differential adapter
//
//   File:       gcr_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::gcr::{
    Drive1541GcrCircuit, Drive1541GcrSignals, Drive1541SpeedZone,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Advance { ticks: u64 },
    ByteReadyEnabled { enabled: bool },
    Flux,
    ReadMode { reading: bool },
    Reset,
    SpeedZone { zone: u8 },
    WriteData { value: u8 },
}

#[derive(Default)]
struct Signals {
    byte_ready: Vec<u8>,
    write_bits: Vec<bool>,
}

impl Drive1541GcrSignals for Signals {
    fn signal_byte_ready(&mut self, data_byte: u8) {
        self.byte_ready.push(data_byte);
    }

    fn write_flux_bit(&mut self, high: bool) {
        self.write_bits.push(high);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    byte_ready: Vec<u8>,
    byte_ready_enabled: bool,
    data_byte: u8,
    reading: bool,
    reference_ticks_until_next_shift: u64,
    speed_zone: u8,
    sync_found: bool,
    weak_flux_ticks_remaining: u64,
    write_bits: Vec<bool>,
    write_data_byte: u8,
}

fn observe(circuit: &Drive1541GcrCircuit, signals: Signals) -> Observation {
    Observation {
        byte_ready: signals.byte_ready,
        byte_ready_enabled: circuit.byte_ready_enabled(),
        data_byte: circuit.data_byte(),
        reading: circuit.reading(),
        reference_ticks_until_next_shift: circuit.reference_ticks_until_next_shift(),
        speed_zone: circuit.speed_zone().get(),
        sync_found: circuit.sync_found(),
        weak_flux_ticks_remaining: circuit.weak_flux_ticks_remaining(),
        write_bits: signals.write_bits,
        write_data_byte: circuit.write_data_byte(),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let operations: Vec<Operation> = serde_json::from_str(&input)?;
    let mut circuit = Drive1541GcrCircuit::new();
    let mut output = Vec::with_capacity(operations.len());
    for operation in operations {
        let mut signals = Signals::default();
        match operation {
            Operation::Advance { ticks } => circuit.advance(ticks, &mut signals)?,
            Operation::ByteReadyEnabled { enabled } => {
                circuit.set_byte_ready_enabled(enabled);
            }
            Operation::Flux => circuit.observe_recorded_flux_reversal(),
            Operation::ReadMode { reading } => circuit.set_read_mode(reading),
            Operation::Reset => circuit.reset(),
            Operation::SpeedZone { zone } => {
                circuit.set_speed_zone(Drive1541SpeedZone::try_from(zone)?);
            }
            Operation::WriteData { value } => circuit.set_write_data_byte(value),
        }
        output.push(observe(&circuit, signals));
    }
    serde_json::to_writer(io::stdout().lock(), &output)?;
    Ok(())
}
