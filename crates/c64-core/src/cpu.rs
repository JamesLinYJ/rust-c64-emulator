// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 6510 可提交架构状态
//
//   文件:       cpu.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

mod interrupt;
mod strict;

pub use interrupt::{CpuInterruptTiming, CpuIrqLine, CpuNmiLine};
pub use strict::{Cpu6510, Cpu6510Error, CpuBus};

pub const CPU_STATUS_CARRY: u8 = 0x01;
pub const CPU_STATUS_ZERO: u8 = 0x02;
pub const CPU_STATUS_INTERRUPT_DISABLE: u8 = 0x04;
pub const CPU_STATUS_DECIMAL: u8 = 0x08;
pub const CPU_STATUS_BREAK: u8 = 0x10;
pub const CPU_STATUS_UNUSED: u8 = 0x20;
pub const CPU_STATUS_OVERFLOW: u8 = 0x40;
pub const CPU_STATUS_NEGATIVE: u8 = 0x80;

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Cpu6510State {
    pub accumulator: u8,
    pub index_x: u8,
    pub index_y: u8,
    pub program_counter: u16,
    pub stack_pointer: u8,
    pub status: u8,
}

impl Default for Cpu6510State {
    fn default() -> Self {
        Self::deterministic_power_on()
    }
}

impl Cpu6510State {
    pub const fn deterministic_power_on() -> Self {
        Self {
            accumulator: 0,
            index_x: 0,
            index_y: 0,
            program_counter: 0,
            stack_pointer: 0xfd,
            status: CPU_STATUS_UNUSED | CPU_STATUS_INTERRUPT_DISABLE,
        }
    }

    pub fn normalize_status(&mut self) {
        self.status |= CPU_STATUS_UNUSED;
    }
}
