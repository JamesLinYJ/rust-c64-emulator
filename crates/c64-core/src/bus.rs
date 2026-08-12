// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 统一总线桥接合约
//
//   文件:       bus.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use crate::{clock::VirtualTimestamp, memory::PhysicalTarget};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusAccessKind {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusMaster {
    Cpu,
    Vic,
    Reu,
    EnhancedDma,
    Cartridge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BusRequest {
    pub timestamp: VirtualTimestamp,
    pub master: BusMaster,
    pub access: BusAccessKind,
    pub address: u32,
    pub value: u8,
    pub target: PhysicalTarget,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BusResponse {
    pub value: u8,
    pub wait_system_cycles: u32,
    pub mapping_changed: bool,
    pub dma_requested: bool,
}

pub trait BusBridge {
    fn transact(&mut self, request: BusRequest) -> BusResponse;

    fn next_external_event(&self) -> Option<VirtualTimestamp>;
}
