// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic CIA differential trace adapter
//
//   File:       cia_trace.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::{Mos6526, Mos6526Model, Mos6526Timing};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    revised: bool,
    processor_clock_hz: u32,
    time_of_day_input_hz: u32,
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Operation {
    Write { address: u16, value: u8 },
    Read { address: u16 },
    Tick { cycles: u64 },
    ClockCycle,
    PulseCount { pulses: u64 },
    SetCountPinHigh { high: bool },
    SetFlagPinHigh { high: bool },
    TickTimeOfDayInput { pulses: u64 },
    PulseSerialClock { high: bool },
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    read_value: Option<u8>,
    port_a_output_pins: u8,
    port_b_output_pins: u8,
    state_flags: u8,
}

fn observe(cia: &Mos6526, read_value: Option<u8>) -> Observation {
    let state_flags = u8::from(cia.interrupt_pending())
        | (u8::from(cia.port_control_output_high()) << 1)
        | (u8::from(cia.serial_clock_output_high()) << 2)
        | (u8::from(cia.serial_data_output_high()) << 3);
    Observation {
        read_value,
        port_a_output_pins: cia.port_a_output_pins(),
        port_b_output_pins: cia.port_b_output_pins(),
        state_flags,
    }
}

fn run_scenario(scenario: Scenario) -> Result<Vec<Observation>, Box<dyn Error>> {
    let model = if scenario.revised {
        Mos6526Model::Revised
    } else {
        Mos6526Model::Original
    };
    let mut cia = Mos6526::new(
        model,
        Mos6526Timing {
            processor_clock_hz: scenario.processor_clock_hz,
            time_of_day_input_hz: scenario.time_of_day_input_hz,
        },
    )?;
    let mut observations = Vec::with_capacity(scenario.operations.len());
    for operation in scenario.operations {
        let read_value = match operation {
            Operation::Write { address, value } => {
                cia.write(address, value);
                None
            }
            Operation::Read { address } => Some(cia.read_pulled_up(address)),
            Operation::Tick { cycles } => {
                cia.tick(cycles);
                None
            }
            Operation::ClockCycle => {
                cia.clock_cycle();
                None
            }
            Operation::PulseCount { pulses } => {
                cia.pulse_count(pulses);
                None
            }
            Operation::SetCountPinHigh { high } => {
                cia.set_count_pin_high(high);
                None
            }
            Operation::SetFlagPinHigh { high } => {
                cia.set_flag_pin_high(high);
                None
            }
            Operation::TickTimeOfDayInput { pulses } => {
                cia.tick_time_of_day_input(pulses);
                None
            }
            Operation::PulseSerialClock { high } => {
                cia.pulse_serial_clock(high);
                None
            }
            Operation::Reset => {
                cia.reset();
                None
            }
        };
        observations.push(observe(&cia, read_value));
    }
    Ok(observations)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let scenarios: Vec<Scenario> = serde_json::from_str(&input)?;
    let results = scenarios
        .into_iter()
        .map(run_scenario)
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_writer(io::stdout().lock(), &results)?;
    Ok(())
}
