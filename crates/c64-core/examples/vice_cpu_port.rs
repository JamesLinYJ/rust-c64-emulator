// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VICE 6510 CPU-port 参考运行器
//
//   文件:       vice_cpu_port.rs
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use std::env;
use std::error::Error;
use std::fs;
use std::io;

use c64_core::{Cpu6510, Cpu6510State, CpuBus, ProcessorPort6510};

const MACHINE_CODE_ENTRY: u16 = 0x080d;
const VICE_TEST_EXIT_PORT: u16 = 0xd7ff;
const MAXIMUM_INSTRUCTIONS: u32 = 10_000;

struct CpuPortBus {
    bytes: Box<[u8]>,
    processor_port: ProcessorPort6510,
    exit_code: Option<u8>,
}

impl CpuPortBus {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x1_0000].into_boxed_slice(),
            processor_port: ProcessorPort6510::new(),
            exit_code: None,
        }
    }

    fn load_prg(&mut self, program: &[u8]) -> Result<(), io::Error> {
        if program.len() < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "VICE CPU-port PRG has no load address",
            ));
        }
        let load_address = usize::from(u16::from_le_bytes([program[0], program[1]]));
        let payload = &program[2..];
        let end = load_address.checked_add(payload.len()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "VICE CPU-port PRG range overflow",
            )
        })?;
        if end > self.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "VICE CPU-port PRG exceeds the 64 KiB address space",
            ));
        }
        self.bytes[load_address..end].copy_from_slice(payload);
        Ok(())
    }
}

impl CpuBus for CpuPortBus {
    fn read(&mut self, address: u16) -> u8 {
        match address {
            0x0000 => self.processor_port.direction_register(),
            0x0001 => self.processor_port.data_register(),
            _ => self.bytes[usize::from(address)],
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000 => {
                self.processor_port.write_direction(value);
            }
            0x0001 => {
                self.processor_port.write_data(value);
            }
            _ => self.bytes[usize::from(address)] = value,
        }
        if address == VICE_TEST_EXIT_PORT {
            self.exit_code = Some(value);
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let program_path = env::args_os().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: vice_cpu_port <cpuport-test1.prg>",
        )
    })?;
    let program = fs::read(program_path)?;
    let mut bus = CpuPortBus::new();
    bus.load_prg(&program)?;
    let mut cpu = Cpu6510::new();
    cpu.restore_state(Cpu6510State {
        program_counter: MACHINE_CODE_ENTRY,
        ..Cpu6510State::deterministic_power_on()
    });

    let mut instructions = 0_u32;
    while bus.exit_code.is_none() && instructions < MAXIMUM_INSTRUCTIONS {
        loop {
            let completed = cpu.clock_cycle(&mut bus)?;
            bus.processor_port.clock_cycle();
            if completed {
                break;
            }
            if cpu.is_jammed() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "VICE CPU-port test entered the 6510 JAM state",
                )
                .into());
            }
        }
        instructions += 1;
    }

    let exit_code = bus.exit_code.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("VICE CPU-port test did not report within {MAXIMUM_INSTRUCTIONS} instructions"),
        )
    })?;
    if exit_code != 0 {
        return Err(io::Error::other(format!(
            "VICE CPU-port test failed with ${exit_code:02x}; screen diagnostic ${:02x}",
            bus.bytes[0x0400]
        ))
        .into());
    }

    println!("PASS Rust VICE 6510 CPU-port test1.prg: {instructions} instructions, exit code $00.");
    Ok(())
}
