// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 1541 integer clock differential adapter
//
//   File:       drive1541_clock_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::drive1541::clock::Drive1541ClockSynchronizer;
use c64_core::devices::drive1541::disk_via::Drive1541DiskVia;
use c64_core::devices::drive1541::iec_via::Drive1541IecVia;
use c64_core::devices::drive1541::machine::Drive1541Machine;
use c64_core::devices::drive1541::mechanism::Drive1541Mechanism;
use c64_core::devices::drive1541::memory::Drive1541Memory;
use c64_core::devices::iec::{IecBus, IecLine};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DifferentialInput {
    host_clock_hz: u64,
    operations: Vec<Operation>,
    rom: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum Operation {
    AdvanceHostCycle,
    AdvanceHostCycles { cycles: u64 },
    ResetClock,
    SetIecLowMask { low_mask: u8 },
}

#[derive(Serialize)]
struct DifferentialOutput {
    snapshots: Vec<ClockSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClockSnapshot {
    cpu: CpuSnapshot,
    elapsed_cycles: u64,
    host_clock_hz: u64,
    iec_bus_low_mask: u8,
    last_data_bus_value: u8,
    last_result: Option<u64>,
    lead_cycles: i128,
    phase_remainder: u64,
    ram: Vec<u8>,
    target_cycles: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CpuSnapshot {
    accumulator: u8,
    at_instruction_boundary: bool,
    index_x: u8,
    index_y: u8,
    jammed: bool,
    program_counter: u16,
    stack_pointer: u8,
    status: u8,
}

fn snapshot(
    clock: Drive1541ClockSynchronizer,
    machine: &Drive1541Machine,
    iec_bus: &IecBus,
    last_result: Option<u64>,
) -> ClockSnapshot {
    let cpu = machine.cpu();
    let state = cpu.state();
    ClockSnapshot {
        cpu: CpuSnapshot {
            accumulator: state.accumulator,
            at_instruction_boundary: cpu.is_at_instruction_boundary(),
            index_x: state.index_x,
            index_y: state.index_y,
            jammed: cpu.is_jammed(),
            program_counter: state.program_counter,
            stack_pointer: state.stack_pointer,
            status: state.status,
        },
        elapsed_cycles: machine.elapsed_cycles(),
        host_clock_hz: clock.host_clock_hz(),
        iec_bus_low_mask: iec_bus.state().low_mask(),
        last_data_bus_value: machine.memory().last_data_bus_value(),
        last_result,
        lead_cycles: clock.lead_cycles(machine),
        phase_remainder: clock.phase_remainder(),
        ram: machine.memory().ram().to_vec(),
        target_cycles: clock.target_cycles(),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input_bytes = String::new();
    io::stdin().read_to_string(&mut input_bytes)?;
    let input: DifferentialInput = serde_json::from_str(&input_bytes)?;
    let mut iec_bus = IecBus::new();
    let host_port = iec_bus.attach()?;
    let mut mechanism = Drive1541Mechanism::new();
    let iec_via = Drive1541IecVia::new(8, &mut iec_bus)?;
    let disk_via = Drive1541DiskVia::new(8, &mut mechanism)?;
    let memory = Drive1541Memory::new(&input.rom, iec_via, disk_via)?;
    let mut machine = Drive1541Machine::new(memory, mechanism);
    let mut clock = Drive1541ClockSynchronizer::try_new(input.host_clock_hz)?;
    let mut snapshots = Vec::with_capacity(input.operations.len() + 1);
    snapshots.push(snapshot(clock, &machine, &iec_bus, None));

    for operation in &input.operations {
        let last_result = match operation {
            Operation::AdvanceHostCycle => {
                Some(clock.advance_host_cycle(&mut machine, &mut iec_bus)?)
            }
            Operation::AdvanceHostCycles { cycles } => {
                Some(clock.advance_host_cycles(&mut machine, &mut iec_bus, *cycles)?)
            }
            Operation::ResetClock => {
                clock.reset_clock(&mut machine);
                None
            }
            Operation::SetIecLowMask { low_mask } => {
                let valid_low_mask =
                    *low_mask & IecLine::ALL.iter().fold(0, |mask, line| mask | line.mask());
                iec_bus.set_port_low_mask(host_port, valid_low_mask)?;
                None
            }
        };
        snapshots.push(snapshot(clock, &machine, &iec_bus, last_result));
    }

    serde_json::to_writer(io::stdout().lock(), &DifferentialOutput { snapshots })?;
    Ok(())
}
