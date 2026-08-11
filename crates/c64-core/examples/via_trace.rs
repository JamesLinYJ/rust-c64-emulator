// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic MOS 6522 differential adapter
//
//   File:       via_trace.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, Read};

use c64_core::devices::via::{self, Mos6522, Mos6522ControlLine};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Operation {
    Clock { cycles: u32 },
    ExternalPorts { port_a: u8, port_b: u8 },
    PortB6 { high: bool },
    Read { address: u16 },
    Reset,
    Signal { high: bool, line: ControlLine },
    Write { address: u16, value: u8 },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ControlLine {
    Ca1,
    Ca2,
    Cb1,
    Cb2,
}

impl From<ControlLine> for Mos6522ControlLine {
    fn from(line: ControlLine) -> Self {
        match line {
            ControlLine::Ca1 => Self::Ca1,
            ControlLine::Ca2 => Self::Ca2,
            ControlLine::Cb1 => Self::Cb1,
            ControlLine::Cb2 => Self::Cb2,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    control_outputs: u8,
    interrupt_enable: u8,
    interrupt_flags: u8,
    interrupt_pending: bool,
    port_a_output_pins: u8,
    port_b_output_pins: u8,
    read_value: Option<u8>,
}

fn observe(via: &mut Mos6522, read_value: Option<u8>) -> Observation {
    Observation {
        control_outputs: u8::from(via.ca2_output_high())
            | (u8::from(via.cb1_output_high()) << 1)
            | (u8::from(via.cb2_output_high()) << 2),
        interrupt_enable: via.read(u16::from(via::register::INTERRUPT_ENABLE)),
        interrupt_flags: via.read(u16::from(via::register::INTERRUPT_FLAGS)),
        interrupt_pending: via.interrupt_pending(),
        port_a_output_pins: via.port_a_output_pins(),
        port_b_output_pins: via.port_b_output_pins(),
        read_value,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let operations: Vec<Operation> = serde_json::from_str(&input)?;
    let mut via = Mos6522::new();
    let mut output = Vec::with_capacity(operations.len());
    for operation in operations {
        let mut read_value = None;
        match operation {
            Operation::Clock { cycles } => {
                via.clock_cycles(cycles);
            }
            Operation::ExternalPorts { port_a, port_b } => {
                via.set_port_a_external_inputs(port_a);
                via.set_port_b_external_inputs(port_b);
            }
            Operation::PortB6 { high } => via.signal_port_b_6(high),
            Operation::Read { address } => read_value = Some(via.read(address)),
            Operation::Reset => via.reset(),
            Operation::Signal { high, line } => via.signal_control_line(line.into(), high),
            Operation::Write { address, value } => via.write(address, value),
        }
        output.push(observe(&mut via, read_value));
    }
    serde_json::to_writer(io::stdout().lock(), &output)?;
    Ok(())
}
