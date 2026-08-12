// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Klaus NMOS 6502 功能参考运行器
//
//   文件:       klaus_6502.rs
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::time::Instant;

use c64_core::{Cpu6510, Cpu6510State, CpuBus};

const START_ADDRESS: u16 = 0x0400;
const SUCCESS_TRAP: u16 = 0x3469;
const MAXIMUM_INSTRUCTIONS: u32 = 40_000_000;

struct FlatBus {
    bytes: Box<[u8]>,
}

impl CpuBus for FlatBus {
    fn read(&mut self, address: u16) -> u8 {
        self.bytes[usize::from(address)]
    }

    fn write(&mut self, address: u16, value: u8) {
        self.bytes[usize::from(address)] = value;
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let image_path = env::args_os().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: klaus_6502 <6502_functional_test.bin>",
        )
    })?;
    let image = fs::read(image_path)?;
    if image.len() != 0x1_0000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Klaus functional image must contain 65536 bytes; received {}",
                image.len()
            ),
        )
        .into());
    }

    let mut bus = FlatBus {
        bytes: image.into_boxed_slice(),
    };
    let mut cpu = Cpu6510::new();
    cpu.restore_state(Cpu6510State {
        program_counter: START_ADDRESS,
        ..Cpu6510State::deterministic_power_on()
    });
    let started_at = Instant::now();

    for instructions in 1..=MAXIMUM_INSTRUCTIONS {
        let previous_program_counter = cpu.state().program_counter;
        loop {
            if cpu.clock_cycle(&mut bus)? {
                break;
            }
            if cpu.is_jammed() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Klaus functional test entered the 6510 JAM state",
                )
                .into());
            }
        }
        if cpu.state().program_counter != previous_program_counter {
            continue;
        }
        if cpu.state().program_counter != SUCCESS_TRAP {
            return Err(io::Error::other(format!(
                "Klaus functional test entered failure trap ${:04x} after {instructions} instructions",
                cpu.state().program_counter
            ))
            .into());
        }

        println!(
            "PASS Rust Klaus 6502 functional test: {instructions} instructions, success trap ${SUCCESS_TRAP:04x}, {:.2} s.",
            started_at.elapsed().as_secs_f64()
        );
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "Klaus functional test did not reach success within {MAXIMUM_INSTRUCTIONS} instructions"
        ),
    )
    .into())
}
