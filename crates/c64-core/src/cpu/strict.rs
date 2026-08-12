// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 周期精确 NMOS 6510 执行核心
//
//   文件:       strict.rs
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use super::{
    CPU_STATUS_BREAK, CPU_STATUS_CARRY, CPU_STATUS_DECIMAL, CPU_STATUS_INTERRUPT_DISABLE,
    CPU_STATUS_NEGATIVE, CPU_STATUS_OVERFLOW, CPU_STATUS_UNUSED, CPU_STATUS_ZERO, Cpu6510State,
    CpuInterruptTiming,
};
use crate::cpu_generated::{OPCODE_OPERATION, OPCODE_PLAN, address_mode, memory_access, operation};
use crate::cpu_generated::{
    OPCODE_PLAN_ACCESS_MASK, OPCODE_PLAN_ACCESS_SHIFT, OPCODE_PLAN_MODE_MASK,
    OPCODE_PLAN_MODE_SHIFT,
};

const CPU_VECTOR_NMI: u16 = 0xfffa;
const CPU_VECTOR_RESET: u16 = 0xfffc;
const CPU_VECTOR_IRQ: u16 = 0xfffe;
const NMOS_UNSTABLE_DATA_MASK: u8 = 0xee;
const JAM_TRANSIENT_ADDRESSES: [u16; 3] = [0xffff, 0xfffe, 0xfffe];
const RUNTIME_FLAG_JAMMED: u8 = 1 << 0;
const RUNTIME_FLAG_PAGE_CROSSED: u8 = 1 << 1;
const RUNTIME_FLAG_INDEXED_STORE_READ_HELD: u8 = 1 << 2;
const RUNTIME_FLAG_NMI_TAKEOVER: u8 = 1 << 3;
const INTERRUPT_SEQUENCE_NONE: u8 = 0;
const INTERRUPT_SEQUENCE_NMI: u8 = 1;
const INTERRUPT_SEQUENCE_IRQ: u8 = 2;

/// CPU 总线的最小逐周期契约。每次调用只允许代表一个真实的 6510 总线周期。
pub trait CpuBus {
    fn read(&mut self, address: u16) -> u8;
    fn write(&mut self, address: u16, value: u8);

    /// 最近一次读是否被 RDY 延长；仅 SHX/SHY 的真实 NMOS 数据掩码规则需要该事实。
    fn read_was_held(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cpu6510Error {
    UnsupportedAddressMode(u8),
    UnsupportedImpliedOperation(u8),
    UnsupportedReadOperation(u8),
    UnsupportedReadModifyWriteOperation(u8),
    UnsupportedStoreOperation(u8),
}

impl fmt::Display for Cpu6510Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedAddressMode(value) => {
                write!(formatter, "unsupported 6510 address mode {value}")
            }
            Self::UnsupportedImpliedOperation(value) => {
                write!(formatter, "unsupported 6510 implied operation {value}")
            }
            Self::UnsupportedReadOperation(value) => {
                write!(formatter, "unsupported 6510 read operation {value}")
            }
            Self::UnsupportedReadModifyWriteOperation(value) => {
                write!(
                    formatter,
                    "unsupported 6510 read-modify-write operation {value}"
                )
            }
            Self::UnsupportedStoreOperation(value) => {
                write!(formatter, "unsupported 6510 store operation {value}")
            }
        }
    }
}

impl std::error::Error for Cpu6510Error {}

/// 无分配、可在任意总线周期暂停的 NMOS 6510 严格执行器。
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Cpu6510 {
    state: Cpu6510State,
    cycles_consumed: u16,
    jam_bus_cycle_index: usize,
    runtime_flags: u8,
    cycle_opcode: u8,
    cycle_operation: u8,
    cycle_address_mode: u8,
    cycle_memory_access: u8,
    cycle_phase: u8,
    cycle_current_cycles: u16,
    cycle_address: u16,
    cycle_base_address: u16,
    cycle_provisional_address: u16,
    cycle_low: u8,
    cycle_data: u8,
    interrupt_sequence: u8,
    interrupt_phase: u8,
    interrupt_timing: CpuInterruptTiming,
    cycle_interrupt_masked_before: bool,
}

impl Default for Cpu6510 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu6510 {
    pub const fn new() -> Self {
        Self {
            state: Cpu6510State::deterministic_power_on(),
            cycles_consumed: 0,
            jam_bus_cycle_index: 0,
            runtime_flags: 0,
            cycle_opcode: 0,
            cycle_operation: operation::NOP,
            cycle_address_mode: address_mode::IMP,
            cycle_memory_access: memory_access::NONE,
            cycle_phase: 0,
            cycle_current_cycles: 0,
            cycle_address: 0,
            cycle_base_address: 0,
            cycle_provisional_address: 0,
            cycle_low: 0,
            cycle_data: 0,
            interrupt_sequence: INTERRUPT_SEQUENCE_NONE,
            interrupt_phase: 0,
            interrupt_timing: CpuInterruptTiming::new(),
            cycle_interrupt_masked_before: false,
        }
    }

    pub const fn state(&self) -> Cpu6510State {
        self.state
    }

    pub fn restore_state(&mut self, state: Cpu6510State) {
        self.state = state;
        self.cycles_consumed = 0;
        self.jam_bus_cycle_index = 0;
        self.runtime_flags = 0;
        self.cycle_phase = 0;
        self.cycle_current_cycles = 0;
        self.interrupt_sequence = INTERRUPT_SEQUENCE_NONE;
        self.interrupt_phase = 0;
        self.interrupt_timing.reset();
        self.cycle_interrupt_masked_before = false;
    }

    pub const fn cycles_consumed(&self) -> u16 {
        self.cycles_consumed
    }

    pub const fn is_jammed(&self) -> bool {
        self.runtime_flags & RUNTIME_FLAG_JAMMED != 0
    }

    pub const fn is_at_instruction_boundary(&self) -> bool {
        !self.is_jammed()
            && self.cycle_phase == 0
            && self.interrupt_sequence == INTERRUPT_SEQUENCE_NONE
    }

    /// 在已提交的指令边界修改下一条取指地址。
    ///
    /// 调试器、监视器和受控媒体装载器可使用此入口；进行中的指令、IRQ/NMI 微序列或
    /// JAM 状态不会被静默丢弃。
    pub fn set_program_counter_at_instruction_boundary(&mut self, program_counter: u16) -> bool {
        if !self.is_at_instruction_boundary() {
            return false;
        }
        self.state.program_counter = program_counter;
        true
    }

    /// The next interrupt-sequence cycle selects IRQ or NMI vectors.
    pub const fn interrupt_vector_selection_pending(&self) -> bool {
        self.interrupt_sequence != INTERRUPT_SEQUENCE_NONE && self.interrupt_phase == 5
    }

    pub fn request_nmi_takeover(&mut self) {
        self.set_runtime_flag(RUNTIME_FLAG_NMI_TAKEOVER, true);
    }

    pub const fn interrupt_disable_is_set(&self) -> bool {
        self.state.status & CPU_STATUS_INTERRUPT_DISABLE != 0
    }

    pub fn can_accept_maskable_interrupt(&self, asserted_cycles: u64) -> bool {
        !self.is_jammed()
            && self
                .interrupt_timing
                .can_accept_maskable_interrupt(asserted_cycles, self.interrupt_disable_is_set())
    }

    pub fn can_accept_non_maskable_interrupt(&self, asserted_cycles: u64) -> bool {
        !self.is_jammed()
            && self
                .interrupt_timing
                .can_accept_non_maskable_interrupt(asserted_cycles)
    }

    pub fn can_take_over_interrupt_sequence_with_nmi(&self, asserted_cycles: u64) -> bool {
        !self.is_jammed()
            && self
                .interrupt_timing
                .can_take_over_interrupt_sequence_with_nmi(asserted_cycles)
    }

    pub fn signal_set_overflow(&mut self) {
        self.state.status |= CPU_STATUS_OVERFLOW;
    }

    /// 从指令边界开始七周期 NMI 微序列；JAM 或非边界状态返回 `false`。
    pub fn begin_non_maskable_interrupt_sequence(&mut self) -> bool {
        self.begin_interrupt_sequence(INTERRUPT_SEQUENCE_NMI)
    }

    /// 从已经通过外部 IRQ 采样裁决的指令边界开始七周期 IRQ 微序列。
    pub fn begin_maskable_interrupt_sequence(&mut self) -> bool {
        self.begin_interrupt_sequence(INTERRUPT_SEQUENCE_IRQ)
    }

    /// 推进恰好一个总线周期；返回 `true` 仅表示形成了新的指令边界。
    ///
    /// # Errors
    ///
    /// 仅当生成的架构表包含严格核心尚未实现的内部操作或寻址类型时返回错误；
    /// 任意访客机字节序列本身都不会触发越界访问或未定义行为。
    pub fn clock_cycle<B: CpuBus>(&mut self, bus: &mut B) -> Result<bool, Cpu6510Error> {
        if self.interrupt_sequence != INTERRUPT_SEQUENCE_NONE {
            return Ok(self.tick_interrupt_sequence(bus));
        }
        if self.is_jammed() {
            self.execute_jammed_bus_cycle(bus);
            return Ok(false);
        }
        if self.cycle_phase == 0 {
            self.set_runtime_flag(RUNTIME_FLAG_INDEXED_STORE_READ_HELD, false);
            self.cycle_interrupt_masked_before = self.interrupt_disable_is_set();
            self.interrupt_timing.begin_instruction();
            let instruction_address = self.state.program_counter;
            self.state.program_counter = self.state.program_counter.wrapping_add(1);
            self.cycle_opcode = bus.read(instruction_address);
            self.cycle_operation = OPCODE_OPERATION[usize::from(self.cycle_opcode)];
            let plan = OPCODE_PLAN[usize::from(self.cycle_opcode)];
            self.cycle_address_mode =
                low_byte((plan >> OPCODE_PLAN_MODE_SHIFT) & OPCODE_PLAN_MODE_MASK);
            self.cycle_memory_access =
                low_byte((plan >> OPCODE_PLAN_ACCESS_SHIFT) & OPCODE_PLAN_ACCESS_MASK);
            self.cycle_phase = 1;
            self.cycle_current_cycles = 1;
            return Ok(false);
        }

        self.cycle_current_cycles += 1;
        if self.cycle_memory_access == memory_access::NONE {
            self.tick_special(bus)
        } else {
            self.tick_addressed(bus)
        }
    }

    /// 执行七个真实 RESET 读周期。A/X/Y 和除 I 外的状态位保持不变。
    pub fn reset<B: CpuBus>(&mut self, bus: &mut B) -> u16 {
        self.set_runtime_flag(RUNTIME_FLAG_JAMMED, false);
        self.jam_bus_cycle_index = 0;
        self.cycle_phase = 0;
        self.cycle_current_cycles = 0;
        self.interrupt_sequence = INTERRUPT_SEQUENCE_NONE;
        self.interrupt_phase = 0;
        self.interrupt_timing.reset();
        self.cycle_interrupt_masked_before = false;
        self.set_runtime_flag(RUNTIME_FLAG_NMI_TAKEOVER, false);
        self.state.status |= CPU_STATUS_INTERRUPT_DISABLE;

        bus.read(self.state.program_counter);
        bus.read(self.state.program_counter);
        for _ in 0..3 {
            bus.read(stack_address(self.state.stack_pointer));
            self.state.stack_pointer = self.state.stack_pointer.wrapping_sub(1);
        }
        let low = bus.read(CPU_VECTOR_RESET);
        let high = bus.read(CPU_VECTOR_RESET + 1);
        self.state.program_counter = word(low, high);
        self.cycles_consumed = 7;
        self.cycles_consumed
    }

    fn begin_interrupt_sequence(&mut self, sequence: u8) -> bool {
        if !self.is_at_instruction_boundary() {
            return false;
        }
        self.interrupt_sequence = sequence;
        self.interrupt_phase = 0;
        self.cycle_current_cycles = 0;
        true
    }

    fn tick_interrupt_sequence<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        self.interrupt_phase += 1;
        self.cycle_current_cycles += 1;
        match self.interrupt_phase {
            1 | 2 => {
                bus.read(self.state.program_counter);
                false
            }
            3 => {
                self.push(bus, high_byte(self.state.program_counter));
                false
            }
            4 => {
                self.push(bus, low_byte(self.state.program_counter));
                false
            }
            5 => {
                self.push(
                    bus,
                    (self.state.status & !CPU_STATUS_BREAK) | CPU_STATUS_UNUSED,
                );
                false
            }
            6 => {
                self.cycle_address = if self.interrupt_sequence == INTERRUPT_SEQUENCE_NMI
                    || self.take_nmi_request()
                {
                    CPU_VECTOR_NMI
                } else {
                    CPU_VECTOR_IRQ
                };
                self.cycle_low = bus.read(self.cycle_address);
                false
            }
            _ => {
                let high = bus.read(self.cycle_address.wrapping_add(1));
                self.state.program_counter = word(self.cycle_low, high);
                self.state.status |= CPU_STATUS_INTERRUPT_DISABLE;
                self.interrupt_sequence = INTERRUPT_SEQUENCE_NONE;
                self.interrupt_phase = 0;
                self.cycles_consumed = self.cycle_current_cycles;
                self.interrupt_timing.complete_interrupt_entry();
                true
            }
        }
    }

    fn finish_instruction(&mut self) -> bool {
        self.cycle_phase = 0;
        self.cycles_consumed = self.cycle_current_cycles;
        self.interrupt_timing.complete_instruction(
            self.interrupt_disable_is_set(),
            self.cycle_interrupt_masked_before,
            self.cycle_opcode,
        );
        true
    }

    fn read_instruction_byte<B: CpuBus>(&mut self, bus: &mut B) -> u8 {
        let address = self.state.program_counter;
        self.state.program_counter = self.state.program_counter.wrapping_add(1);
        bus.read(address)
    }

    fn tick_addressed<B: CpuBus>(&mut self, bus: &mut B) -> Result<bool, Cpu6510Error> {
        match self.cycle_address_mode {
            address_mode::IMM => {
                self.cycle_data = self.read_instruction_byte(bus);
                self.apply_read(self.cycle_data)?;
                Ok(self.finish_instruction())
            }
            address_mode::ZP => self.tick_zero_page(bus, false, false),
            address_mode::ZPX => self.tick_zero_page(bus, true, false),
            address_mode::ZPY => self.tick_zero_page(bus, false, true),
            address_mode::ABS => self.tick_absolute(bus, false, false),
            address_mode::ABSX => self.tick_absolute(bus, true, false),
            address_mode::ABSY => self.tick_absolute(bus, false, true),
            address_mode::INDX => self.tick_indirect_x(bus),
            address_mode::INDY => self.tick_indirect_y(bus),
            value => Err(Cpu6510Error::UnsupportedAddressMode(value)),
        }
    }

    fn tick_zero_page<B: CpuBus>(
        &mut self,
        bus: &mut B,
        index_x: bool,
        index_y: bool,
    ) -> Result<bool, Cpu6510Error> {
        let indexed = index_x || index_y;
        if self.cycle_phase == 1 {
            self.cycle_base_address = u16::from(self.read_instruction_byte(bus));
            self.cycle_address = self.cycle_base_address;
            self.cycle_phase = 2;
            return Ok(false);
        }
        if indexed && self.cycle_phase == 2 {
            bus.read(self.cycle_base_address);
            let index = if index_x {
                self.state.index_x
            } else {
                self.state.index_y
            };
            self.cycle_address = u16::from(low_byte(self.cycle_base_address).wrapping_add(index));
            self.cycle_phase = 3;
            return Ok(false);
        }
        self.tick_effective_address(bus, if indexed { 3 } else { 2 })
    }

    fn tick_absolute<B: CpuBus>(
        &mut self,
        bus: &mut B,
        index_x: bool,
        index_y: bool,
    ) -> Result<bool, Cpu6510Error> {
        let indexed = index_x || index_y;
        if self.cycle_phase == 1 {
            self.cycle_low = self.read_instruction_byte(bus);
            self.cycle_phase = 2;
            return Ok(false);
        }
        if self.cycle_phase == 2 {
            let high = self.read_instruction_byte(bus);
            self.cycle_base_address = word(self.cycle_low, high);
            let index = if index_x {
                self.state.index_x
            } else if index_y {
                self.state.index_y
            } else {
                0
            };
            self.cycle_address = self.cycle_base_address.wrapping_add(u16::from(index));
            self.cycle_provisional_address =
                (self.cycle_base_address & 0xff00) | u16::from(low_byte(self.cycle_address));
            self.set_runtime_flag(
                RUNTIME_FLAG_PAGE_CROSSED,
                page_crossed(self.cycle_base_address, self.cycle_address),
            );
            self.cycle_phase = 3;
            return Ok(false);
        }
        if indexed
            && self.cycle_phase == 3
            && (self.cycle_memory_access != memory_access::READ
                || self.runtime_flag(RUNTIME_FLAG_PAGE_CROSSED))
        {
            bus.read(self.cycle_provisional_address);
            if self.cycle_operation == operation::XAS || self.cycle_operation == operation::SAY {
                self.set_runtime_flag(RUNTIME_FLAG_INDEXED_STORE_READ_HELD, bus.read_was_held());
            }
            self.cycle_phase = 4;
            return Ok(false);
        }
        let data_phase = if indexed
            && (self.cycle_memory_access != memory_access::READ
                || self.runtime_flag(RUNTIME_FLAG_PAGE_CROSSED))
        {
            4
        } else {
            3
        };
        self.tick_effective_address(bus, data_phase)
    }

    fn tick_indirect_x<B: CpuBus>(&mut self, bus: &mut B) -> Result<bool, Cpu6510Error> {
        if self.cycle_phase == 1 {
            self.cycle_base_address = u16::from(self.read_instruction_byte(bus));
            self.cycle_phase = 2;
            return Ok(false);
        }
        if self.cycle_phase == 2 {
            bus.read(self.cycle_base_address);
            self.cycle_base_address =
                u16::from(low_byte(self.cycle_base_address).wrapping_add(self.state.index_x));
            self.cycle_phase = 3;
            return Ok(false);
        }
        if self.cycle_phase == 3 {
            self.cycle_low = bus.read(self.cycle_base_address);
            self.cycle_phase = 4;
            return Ok(false);
        }
        if self.cycle_phase == 4 {
            let high = bus.read(u16::from(low_byte(self.cycle_base_address).wrapping_add(1)));
            self.cycle_address = word(self.cycle_low, high);
            self.cycle_phase = 5;
            return Ok(false);
        }
        self.tick_effective_address(bus, 5)
    }

    fn tick_indirect_y<B: CpuBus>(&mut self, bus: &mut B) -> Result<bool, Cpu6510Error> {
        if self.cycle_phase == 1 {
            self.cycle_base_address = u16::from(self.read_instruction_byte(bus));
            self.cycle_phase = 2;
            return Ok(false);
        }
        if self.cycle_phase == 2 {
            self.cycle_low = bus.read(self.cycle_base_address);
            self.cycle_phase = 3;
            return Ok(false);
        }
        if self.cycle_phase == 3 {
            let high = bus.read(u16::from(low_byte(self.cycle_base_address).wrapping_add(1)));
            self.cycle_base_address = word(self.cycle_low, high);
            self.cycle_address = self
                .cycle_base_address
                .wrapping_add(u16::from(self.state.index_y));
            self.cycle_provisional_address =
                (self.cycle_base_address & 0xff00) | u16::from(low_byte(self.cycle_address));
            self.set_runtime_flag(
                RUNTIME_FLAG_PAGE_CROSSED,
                page_crossed(self.cycle_base_address, self.cycle_address),
            );
            self.cycle_phase = 4;
            return Ok(false);
        }
        if self.cycle_phase == 4
            && (self.cycle_memory_access != memory_access::READ
                || self.runtime_flag(RUNTIME_FLAG_PAGE_CROSSED))
        {
            bus.read(self.cycle_provisional_address);
            self.cycle_phase = 5;
            return Ok(false);
        }
        let data_phase = if self.cycle_memory_access != memory_access::READ
            || self.runtime_flag(RUNTIME_FLAG_PAGE_CROSSED)
        {
            5
        } else {
            4
        };
        self.tick_effective_address(bus, data_phase)
    }

    fn tick_effective_address<B: CpuBus>(
        &mut self,
        bus: &mut B,
        data_phase: u8,
    ) -> Result<bool, Cpu6510Error> {
        if self.cycle_phase == data_phase {
            if self.cycle_memory_access == memory_access::READ {
                self.cycle_data = bus.read(self.cycle_address);
                self.apply_read(self.cycle_data)?;
                return Ok(self.finish_instruction());
            }
            if self.cycle_memory_access == memory_access::WRITE {
                let (address, value) = self.store_address_and_value()?;
                bus.write(address, value);
                return Ok(self.finish_instruction());
            }
            self.cycle_data = bus.read(self.cycle_address);
            self.cycle_phase += 1;
            return Ok(false);
        }
        if self.cycle_phase == data_phase + 1 {
            bus.write(self.cycle_address, self.cycle_data);
            self.cycle_phase += 1;
            return Ok(false);
        }
        let transformed = self.prepare_read_modify_write(self.cycle_data)?;
        bus.write(self.cycle_address, transformed);
        self.complete_read_modify_write(transformed);
        Ok(self.finish_instruction())
    }

    fn store_address_and_value(&mut self) -> Result<(u16, u8), Cpu6510Error> {
        let mut value = match self.cycle_operation {
            operation::STA => self.state.accumulator,
            operation::STX | operation::XAS => self.state.index_x,
            operation::STY | operation::SAY => self.state.index_y,
            operation::AXS_STORE | operation::AXA => self.state.accumulator & self.state.index_x,
            operation::TAS => {
                self.state.stack_pointer = self.state.accumulator & self.state.index_x;
                self.state.stack_pointer
            }
            other => return Err(Cpu6510Error::UnsupportedStoreOperation(other)),
        };

        let mut write_address = self.cycle_address;
        if matches!(
            self.cycle_operation,
            operation::AXA | operation::TAS | operation::XAS | operation::SAY
        ) {
            let high_byte_mask = high_byte(self.cycle_base_address).wrapping_add(1);
            let address_high_value = value & high_byte_mask;
            let data_mask_drops = self.runtime_flag(RUNTIME_FLAG_INDEXED_STORE_READ_HELD)
                && matches!(self.cycle_operation, operation::XAS | operation::SAY);
            if !data_mask_drops {
                value = address_high_value;
            }
            if page_crossed(self.cycle_base_address, self.cycle_address) {
                write_address =
                    (u16::from(address_high_value) << 8) | u16::from(low_byte(self.cycle_address));
            }
        }
        Ok((write_address, value))
    }

    fn tick_special<B: CpuBus>(&mut self, bus: &mut B) -> Result<bool, Cpu6510Error> {
        match self.cycle_operation {
            operation::BRK => Ok(self.tick_break(bus)),
            operation::JAM => {
                bus.read(self.state.program_counter);
                self.set_runtime_flag(RUNTIME_FLAG_JAMMED, true);
                self.jam_bus_cycle_index = 0;
                self.cycle_phase = 0;
                self.cycles_consumed = self.cycle_current_cycles;
                Ok(false)
            }
            operation::JSR => Ok(self.tick_jump_subroutine(bus)),
            operation::JMP => Ok(self.tick_jump(bus)),
            operation::RTS => Ok(self.tick_return_subroutine(bus)),
            operation::RTI => Ok(self.tick_return_interrupt(bus)),
            operation::PHA | operation::PHP => Ok(self.tick_push(bus)),
            operation::PLA | operation::PLP => Ok(self.tick_pull(bus)),
            _ if self.cycle_address_mode == address_mode::REL => Ok(self.tick_branch(bus)),
            _ => {
                bus.read(self.state.program_counter);
                self.apply_implied()?;
                Ok(self.finish_instruction())
            }
        }
    }

    fn tick_break<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            bus.read(self.state.program_counter);
            self.state.program_counter = self.state.program_counter.wrapping_add(1);
            self.cycle_phase = 2;
            return false;
        }
        if self.cycle_phase == 2 {
            self.push(bus, high_byte(self.state.program_counter));
            self.cycle_phase = 3;
            return false;
        }
        if self.cycle_phase == 3 {
            self.push(bus, low_byte(self.state.program_counter));
            self.cycle_phase = 4;
            return false;
        }
        if self.cycle_phase == 4 {
            self.push(
                bus,
                self.state.status | CPU_STATUS_BREAK | CPU_STATUS_UNUSED,
            );
            self.cycle_phase = 5;
            return false;
        }
        if self.cycle_phase == 5 {
            self.cycle_address = if self.take_nmi_request() {
                CPU_VECTOR_NMI
            } else {
                CPU_VECTOR_IRQ
            };
            self.cycle_low = bus.read(self.cycle_address);
            self.cycle_phase = 6;
            return false;
        }
        let high = bus.read(self.cycle_address.wrapping_add(1));
        self.state.program_counter = word(self.cycle_low, high);
        self.state.status |= CPU_STATUS_INTERRUPT_DISABLE;
        self.finish_instruction()
    }

    fn tick_jump_subroutine<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            self.cycle_low = bus.read(self.state.program_counter);
            self.state.program_counter = self.state.program_counter.wrapping_add(1);
            self.cycle_phase = 2;
            return false;
        }
        if self.cycle_phase == 2 {
            bus.read(stack_address(self.state.stack_pointer));
            self.cycle_phase = 3;
            return false;
        }
        if self.cycle_phase == 3 {
            self.push(bus, high_byte(self.state.program_counter));
            self.cycle_phase = 4;
            return false;
        }
        if self.cycle_phase == 4 {
            self.push(bus, low_byte(self.state.program_counter));
            self.cycle_phase = 5;
            return false;
        }
        let high = bus.read(self.state.program_counter);
        self.state.program_counter = word(self.cycle_low, high);
        self.finish_instruction()
    }

    fn tick_jump<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            self.cycle_low = self.read_instruction_byte(bus);
            self.cycle_phase = 2;
            return false;
        }
        if self.cycle_phase == 2 {
            let high = bus.read(self.state.program_counter);
            self.cycle_address = word(self.cycle_low, high);
            if self.cycle_address_mode == address_mode::ABS {
                self.state.program_counter = self.cycle_address;
                return self.finish_instruction();
            }
            self.cycle_phase = 3;
            return false;
        }
        if self.cycle_phase == 3 {
            self.cycle_low = bus.read(self.cycle_address);
            self.cycle_phase = 4;
            return false;
        }
        let high_address = if low_byte(self.cycle_address) == 0xff {
            self.cycle_address & 0xff00
        } else {
            self.cycle_address.wrapping_add(1)
        };
        let high = bus.read(high_address);
        self.state.program_counter = word(self.cycle_low, high);
        self.finish_instruction()
    }

    fn tick_return_subroutine<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            bus.read(self.state.program_counter);
            self.cycle_phase = 2;
            return false;
        }
        if self.cycle_phase == 2 {
            bus.read(stack_address(self.state.stack_pointer));
            self.cycle_phase = 3;
            return false;
        }
        if self.cycle_phase == 3 {
            self.state.stack_pointer = self.state.stack_pointer.wrapping_add(1);
            self.cycle_low = bus.read(stack_address(self.state.stack_pointer));
            self.cycle_phase = 4;
            return false;
        }
        if self.cycle_phase == 4 {
            self.state.stack_pointer = self.state.stack_pointer.wrapping_add(1);
            let high = bus.read(stack_address(self.state.stack_pointer));
            self.cycle_address = word(self.cycle_low, high);
            self.cycle_phase = 5;
            return false;
        }
        bus.read(self.cycle_address);
        self.state.program_counter = self.cycle_address.wrapping_add(1);
        self.finish_instruction()
    }

    fn tick_return_interrupt<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            bus.read(self.state.program_counter);
            self.cycle_phase = 2;
            return false;
        }
        if self.cycle_phase == 2 {
            bus.read(stack_address(self.state.stack_pointer));
            self.cycle_phase = 3;
            return false;
        }
        if self.cycle_phase == 3 {
            self.state.stack_pointer = self.state.stack_pointer.wrapping_add(1);
            let value = bus.read(stack_address(self.state.stack_pointer));
            self.state.status = (value & !CPU_STATUS_BREAK) | CPU_STATUS_UNUSED;
            self.cycle_phase = 4;
            return false;
        }
        if self.cycle_phase == 4 {
            self.state.stack_pointer = self.state.stack_pointer.wrapping_add(1);
            self.cycle_low = bus.read(stack_address(self.state.stack_pointer));
            self.cycle_phase = 5;
            return false;
        }
        self.state.stack_pointer = self.state.stack_pointer.wrapping_add(1);
        let high = bus.read(stack_address(self.state.stack_pointer));
        self.state.program_counter = word(self.cycle_low, high);
        self.finish_instruction()
    }

    fn tick_push<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            bus.read(self.state.program_counter);
            self.cycle_phase = 2;
            return false;
        }
        let value = if self.cycle_operation == operation::PHA {
            self.state.accumulator
        } else {
            self.state.status | CPU_STATUS_BREAK | CPU_STATUS_UNUSED
        };
        self.push(bus, value);
        self.finish_instruction()
    }

    fn tick_pull<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            bus.read(self.state.program_counter);
            self.cycle_phase = 2;
            return false;
        }
        if self.cycle_phase == 2 {
            bus.read(stack_address(self.state.stack_pointer));
            self.cycle_phase = 3;
            return false;
        }
        self.state.stack_pointer = self.state.stack_pointer.wrapping_add(1);
        let value = bus.read(stack_address(self.state.stack_pointer));
        if self.cycle_operation == operation::PLA {
            self.state.accumulator = value;
            self.set_zero_negative(value);
        } else {
            self.state.status = (value & !CPU_STATUS_BREAK) | CPU_STATUS_UNUSED;
        }
        self.finish_instruction()
    }

    fn tick_branch<B: CpuBus>(&mut self, bus: &mut B) -> bool {
        if self.cycle_phase == 1 {
            self.cycle_data = self.read_instruction_byte(bus);
            if !self.is_branch_taken() {
                return self.finish_instruction();
            }
            self.interrupt_timing.delay_interrupt_for_taken_branch();
            self.cycle_base_address = self.state.program_counter;
            let offset = i16::from(i8::from_ne_bytes([self.cycle_data]));
            self.cycle_address = self.state.program_counter.wrapping_add_signed(offset);
            self.set_runtime_flag(
                RUNTIME_FLAG_PAGE_CROSSED,
                page_crossed(self.cycle_base_address, self.cycle_address),
            );
            self.cycle_phase = 2;
            return false;
        }
        if self.cycle_phase == 2 {
            bus.read(self.cycle_base_address);
            if !self.runtime_flag(RUNTIME_FLAG_PAGE_CROSSED) {
                self.state.program_counter = self.cycle_address;
                return self.finish_instruction();
            }
            self.cycle_phase = 3;
            return false;
        }
        bus.read((self.cycle_base_address & 0xff00) | u16::from(low_byte(self.cycle_address)));
        self.state.program_counter = self.cycle_address;
        self.finish_instruction()
    }

    fn is_branch_taken(&self) -> bool {
        match self.cycle_operation {
            operation::BPL => self.state.status & CPU_STATUS_NEGATIVE == 0,
            operation::BMI => self.state.status & CPU_STATUS_NEGATIVE != 0,
            operation::BVC => self.state.status & CPU_STATUS_OVERFLOW == 0,
            operation::BVS => self.state.status & CPU_STATUS_OVERFLOW != 0,
            operation::BCC => self.state.status & CPU_STATUS_CARRY == 0,
            operation::BCS => self.state.status & CPU_STATUS_CARRY != 0,
            operation::BNE => self.state.status & CPU_STATUS_ZERO == 0,
            operation::BEQ => self.state.status & CPU_STATUS_ZERO != 0,
            _ => false,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn apply_implied(&mut self) -> Result<(), Cpu6510Error> {
        match self.cycle_operation {
            operation::NOP => {}
            operation::ASL => self.state.accumulator = self.asl(self.state.accumulator),
            operation::ROL => self.state.accumulator = self.rol(self.state.accumulator),
            operation::LSR => self.state.accumulator = self.lsr(self.state.accumulator),
            operation::ROR => self.state.accumulator = self.ror(self.state.accumulator),
            operation::CLC => self.state.status &= !CPU_STATUS_CARRY,
            operation::SEC => self.state.status |= CPU_STATUS_CARRY,
            operation::CLI => self.state.status &= !CPU_STATUS_INTERRUPT_DISABLE,
            operation::SEI => self.state.status |= CPU_STATUS_INTERRUPT_DISABLE,
            operation::CLV => self.state.status &= !CPU_STATUS_OVERFLOW,
            operation::CLD => self.state.status &= !CPU_STATUS_DECIMAL,
            operation::SED => self.state.status |= CPU_STATUS_DECIMAL,
            operation::DEY => {
                self.state.index_y = self.state.index_y.wrapping_sub(1);
                self.set_zero_negative(self.state.index_y);
            }
            operation::INY => {
                self.state.index_y = self.state.index_y.wrapping_add(1);
                self.set_zero_negative(self.state.index_y);
            }
            operation::DEX => {
                self.state.index_x = self.state.index_x.wrapping_sub(1);
                self.set_zero_negative(self.state.index_x);
            }
            operation::INX => {
                self.state.index_x = self.state.index_x.wrapping_add(1);
                self.set_zero_negative(self.state.index_x);
            }
            operation::TXA => {
                self.state.accumulator = self.state.index_x;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::TYA => {
                self.state.accumulator = self.state.index_y;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::TAY => {
                self.state.index_y = self.state.accumulator;
                self.set_zero_negative(self.state.index_y);
            }
            operation::TAX => {
                self.state.index_x = self.state.accumulator;
                self.set_zero_negative(self.state.index_x);
            }
            operation::TXS => self.state.stack_pointer = self.state.index_x,
            operation::TSX => {
                self.state.index_x = self.state.stack_pointer;
                self.set_zero_negative(self.state.index_x);
            }
            other => return Err(Cpu6510Error::UnsupportedImpliedOperation(other)),
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_read(&mut self, value: u8) -> Result<(), Cpu6510Error> {
        match self.cycle_operation {
            operation::NOP => {}
            operation::ORA => {
                self.state.accumulator |= value;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::AND => {
                self.state.accumulator &= value;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::EOR => {
                self.state.accumulator ^= value;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::ADC => self.add(value),
            operation::SBC => self.subtract(value),
            operation::BIT => {
                self.state.status &= !(CPU_STATUS_ZERO | CPU_STATUS_OVERFLOW | CPU_STATUS_NEGATIVE);
                self.state.status |= value & (CPU_STATUS_OVERFLOW | CPU_STATUS_NEGATIVE);
                if self.state.accumulator & value == 0 {
                    self.state.status |= CPU_STATUS_ZERO;
                }
            }
            operation::LDA => {
                self.state.accumulator = value;
                self.set_zero_negative(value);
            }
            operation::LDX => {
                self.state.index_x = value;
                self.set_zero_negative(value);
            }
            operation::LDY => {
                self.state.index_y = value;
                self.set_zero_negative(value);
            }
            operation::LAX => {
                self.state.accumulator = value;
                self.state.index_x = value;
                self.set_zero_negative(value);
            }
            operation::LAS => {
                self.state.accumulator = value & self.state.stack_pointer;
                self.state.index_x = self.state.accumulator;
                self.state.stack_pointer = self.state.accumulator;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::CMP => self.compare(self.state.accumulator, value),
            operation::CPX => self.compare(self.state.index_x, value),
            operation::CPY => self.compare(self.state.index_y, value),
            operation::ANC => {
                self.state.accumulator &= value;
                self.set_zero_negative(self.state.accumulator);
                self.state.status &= !CPU_STATUS_CARRY;
                if self.state.status & CPU_STATUS_NEGATIVE != 0 {
                    self.state.status |= CPU_STATUS_CARRY;
                }
            }
            operation::ALR => {
                self.state.accumulator &= value;
                self.set_zero_negative(self.state.accumulator);
                self.state.accumulator = self.lsr(self.state.accumulator);
            }
            operation::ARR => self.arr(value),
            operation::XAA => {
                self.state.accumulator =
                    (self.state.accumulator | NMOS_UNSTABLE_DATA_MASK) & self.state.index_x & value;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::OAL => {
                self.state.accumulator = (self.state.accumulator | NMOS_UNSTABLE_DATA_MASK) & value;
                self.state.index_x = self.state.accumulator;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::SAX_SUBTRACT => {
                let difference =
                    i16::from(self.state.accumulator & self.state.index_x) - i16::from(value);
                self.state.status &= !CPU_STATUS_CARRY;
                if difference >= 0 {
                    self.state.status |= CPU_STATUS_CARRY;
                }
                self.state.index_x = difference.to_le_bytes()[0];
                self.set_zero_negative(self.state.index_x);
            }
            other => return Err(Cpu6510Error::UnsupportedReadOperation(other)),
        }
        Ok(())
    }

    fn prepare_read_modify_write(&mut self, value: u8) -> Result<u8, Cpu6510Error> {
        match self.cycle_operation {
            operation::ASL | operation::ASO => Ok(self.asl(value)),
            operation::ROL | operation::RLA => Ok(self.rol(value)),
            operation::LSR | operation::LSE => Ok(self.lsr(value)),
            operation::ROR | operation::RRA => Ok(self.ror(value)),
            operation::DEC => Ok(self.decrement(value)),
            operation::INC | operation::INS => Ok(self.increment(value)),
            operation::DCM => Ok(value.wrapping_sub(1)),
            other => Err(Cpu6510Error::UnsupportedReadModifyWriteOperation(other)),
        }
    }

    fn complete_read_modify_write(&mut self, value: u8) {
        match self.cycle_operation {
            operation::ASO => {
                self.state.accumulator |= value;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::RLA => {
                self.state.accumulator &= value;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::LSE => {
                self.state.accumulator ^= value;
                self.set_zero_negative(self.state.accumulator);
            }
            operation::RRA => self.add(value),
            operation::DCM => self.compare(self.state.accumulator, value),
            operation::INS => self.subtract(value),
            _ => {}
        }
    }

    fn set_zero_negative(&mut self, value: u8) {
        self.state.status &= !(CPU_STATUS_ZERO | CPU_STATUS_NEGATIVE);
        if value == 0 {
            self.state.status |= CPU_STATUS_ZERO;
        }
        self.state.status |= value & CPU_STATUS_NEGATIVE;
    }

    fn asl(&mut self, value: u8) -> u8 {
        self.state.status &= !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_NEGATIVE);
        self.state.status |= value >> 7;
        let result = value.wrapping_shl(1);
        self.set_zero_negative(result);
        result
    }

    fn lsr(&mut self, value: u8) -> u8 {
        self.state.status &= !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_NEGATIVE);
        self.state.status |= value & CPU_STATUS_CARRY;
        let result = value >> 1;
        self.set_zero_negative(result);
        result
    }

    fn rol(&mut self, value: u8) -> u8 {
        let carry_in = self.state.status & CPU_STATUS_CARRY;
        self.state.status &= !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_NEGATIVE);
        self.state.status |= value >> 7;
        let result = value.wrapping_shl(1) | carry_in;
        self.set_zero_negative(result);
        result
    }

    fn ror(&mut self, value: u8) -> u8 {
        let carry_in = self.state.status & CPU_STATUS_CARRY;
        self.state.status &= !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_NEGATIVE);
        self.state.status |= value & CPU_STATUS_CARRY;
        let result = (value >> 1) | (carry_in << 7);
        self.set_zero_negative(result);
        result
    }

    fn increment(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        self.set_zero_negative(result);
        result
    }

    fn decrement(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        self.set_zero_negative(result);
        result
    }

    fn add(&mut self, value: u8) {
        let accumulator = self.state.accumulator;
        let carry = u16::from(self.state.status & CPU_STATUS_CARRY);
        let sum = u16::from(accumulator) + u16::from(value) + carry;
        let binary_result = low_byte(sum);
        self.state.status &=
            !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_OVERFLOW | CPU_STATUS_NEGATIVE);

        if self.state.status & CPU_STATUS_DECIMAL != 0 {
            let mut low_nibble_sum =
                u16::from(accumulator & 0x0f) + u16::from(value & 0x0f) + carry;
            if low_nibble_sum >= 0x0a {
                low_nibble_sum = ((low_nibble_sum + 0x06) & 0x0f) + 0x10;
            }
            let mut decimal_intermediate =
                u16::from(accumulator & 0xf0) + u16::from(value & 0xf0) + low_nibble_sum;
            if binary_result == 0 {
                self.state.status |= CPU_STATUS_ZERO;
            }
            if low_byte(decimal_intermediate) & CPU_STATUS_NEGATIVE != 0 {
                self.state.status |= CPU_STATUS_NEGATIVE;
            }
            if (!(accumulator ^ value)
                & (accumulator ^ low_byte(decimal_intermediate))
                & CPU_STATUS_NEGATIVE)
                != 0
            {
                self.state.status |= CPU_STATUS_OVERFLOW;
            }
            if decimal_intermediate >= 0xa0 {
                decimal_intermediate += 0x60;
            }
            if decimal_intermediate > 0xff {
                self.state.status |= CPU_STATUS_CARRY;
            }
            self.state.accumulator = low_byte(decimal_intermediate);
            return;
        }

        if (!(accumulator ^ value) & (accumulator ^ binary_result) & CPU_STATUS_NEGATIVE) != 0 {
            self.state.status |= CPU_STATUS_OVERFLOW;
        }
        if sum > 0xff {
            self.state.status |= CPU_STATUS_CARRY;
        }
        self.state.accumulator = binary_result;
        self.set_zero_negative(binary_result);
    }

    fn subtract(&mut self, value: u8) {
        let accumulator = self.state.accumulator;
        let borrow = i16::from(u8::from(self.state.status & CPU_STATUS_CARRY == 0));
        let difference = i16::from(accumulator) - i16::from(value) - borrow;
        let binary_result = difference.to_le_bytes()[0];
        self.state.status &=
            !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_OVERFLOW | CPU_STATUS_NEGATIVE);
        if ((accumulator ^ binary_result) & (accumulator ^ value) & CPU_STATUS_NEGATIVE) != 0 {
            self.state.status |= CPU_STATUS_OVERFLOW;
        }
        if difference >= 0 {
            self.state.status |= CPU_STATUS_CARRY;
        }

        if self.state.status & CPU_STATUS_DECIMAL != 0 {
            let mut low_nibble_difference =
                i16::from(accumulator & 0x0f) - i16::from(value & 0x0f) - borrow;
            let mut high_nibble_difference = i16::from(accumulator >> 4) - i16::from(value >> 4);
            if low_nibble_difference < 0 {
                low_nibble_difference -= 0x06;
                high_nibble_difference -= 1;
            }
            if high_nibble_difference < 0 {
                high_nibble_difference -= 0x06;
            }
            self.state.accumulator = (high_nibble_difference << 4).to_le_bytes()[0] & 0xf0
                | low_nibble_difference.to_le_bytes()[0] & 0x0f;
            self.set_zero_negative(binary_result);
            return;
        }

        self.state.accumulator = binary_result;
        self.set_zero_negative(binary_result);
    }

    fn arr(&mut self, immediate: u8) {
        let and_result = self.state.accumulator & immediate;
        let carry_in = self.state.status & CPU_STATUS_CARRY;
        // `(A & imm | C << 8) >>> 1` 的位 7 来自旧 C。
        let mut rotated_result = (and_result >> 1) | (carry_in << 7);
        self.state.status &=
            !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_OVERFLOW | CPU_STATUS_NEGATIVE);
        if carry_in != 0 {
            self.state.status |= CPU_STATUS_NEGATIVE;
        }
        if rotated_result == 0 {
            self.state.status |= CPU_STATUS_ZERO;
        }
        if (rotated_result ^ and_result) & CPU_STATUS_OVERFLOW != 0 {
            self.state.status |= CPU_STATUS_OVERFLOW;
        }

        if self.state.status & CPU_STATUS_DECIMAL != 0 {
            if (and_result & 0x0f) + (and_result & CPU_STATUS_CARRY) > 0x05 {
                rotated_result =
                    (rotated_result & 0xf0) | (rotated_result.wrapping_add(0x06) & 0x0f);
            }
            if (and_result & 0xf0) + (and_result & 0x10) > 0x50 {
                rotated_result =
                    (rotated_result & 0x0f) | (rotated_result.wrapping_add(0x60) & 0xf0);
                self.state.status |= CPU_STATUS_CARRY;
            }
        } else if rotated_result & 0x40 != 0 {
            self.state.status |= CPU_STATUS_CARRY;
        }
        self.state.accumulator = rotated_result;
    }

    fn compare(&mut self, register: u8, value: u8) {
        let difference = register.wrapping_sub(value);
        self.state.status &= !(CPU_STATUS_CARRY | CPU_STATUS_ZERO | CPU_STATUS_NEGATIVE);
        if register >= value {
            self.state.status |= CPU_STATUS_CARRY;
        }
        self.set_zero_negative(difference);
    }

    fn push<B: CpuBus>(&mut self, bus: &mut B, value: u8) {
        bus.write(stack_address(self.state.stack_pointer), value);
        self.state.stack_pointer = self.state.stack_pointer.wrapping_sub(1);
    }

    const fn runtime_flag(&self, flag: u8) -> bool {
        self.runtime_flags & flag != 0
    }

    fn set_runtime_flag(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.runtime_flags |= flag;
        } else {
            self.runtime_flags &= !flag;
        }
    }

    fn take_nmi_request(&mut self) -> bool {
        let requested = self.runtime_flag(RUNTIME_FLAG_NMI_TAKEOVER);
        self.set_runtime_flag(RUNTIME_FLAG_NMI_TAKEOVER, false);
        requested
    }

    fn execute_jammed_bus_cycle<B: CpuBus>(&mut self, bus: &mut B) {
        let address = JAM_TRANSIENT_ADDRESSES
            .get(self.jam_bus_cycle_index)
            .copied()
            .unwrap_or(0xffff);
        bus.read(address);
        if self.jam_bus_cycle_index < JAM_TRANSIENT_ADDRESSES.len() {
            self.jam_bus_cycle_index += 1;
        }
        self.cycles_consumed = 1;
    }
}

const fn low_byte(value: u16) -> u8 {
    value.to_le_bytes()[0]
}

const fn high_byte(value: u16) -> u8 {
    value.to_le_bytes()[1]
}

const fn word(low: u8, high: u8) -> u16 {
    u16::from_le_bytes([low, high])
}

const fn stack_address(stack_pointer: u8) -> u16 {
    u16::from_le_bytes([stack_pointer, 0x01])
}

const fn page_crossed(base: u16, address: u16) -> bool {
    (base ^ address) & 0x0100 != 0
}

#[cfg(test)]
mod tests {
    use super::{
        CPU_STATUS_BREAK, CPU_STATUS_CARRY, CPU_STATUS_INTERRUPT_DISABLE, CPU_STATUS_UNUSED,
    };
    use super::{Cpu6510, Cpu6510State, CpuBus};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Kind {
        Read,
        Write,
    }

    struct TestBus {
        bytes: Box<[u8]>,
        accesses: Vec<(Kind, u16, u8)>,
    }

    impl TestBus {
        fn new() -> Self {
            Self {
                bytes: vec![0; 0x1_0000].into_boxed_slice(),
                accesses: Vec::new(),
            }
        }
    }

    impl CpuBus for TestBus {
        fn read(&mut self, address: u16) -> u8 {
            let value = self.bytes[usize::from(address)];
            self.accesses.push((Kind::Read, address, value));
            value
        }

        fn write(&mut self, address: u16, value: u8) {
            self.bytes[usize::from(address)] = value;
            self.accesses.push((Kind::Write, address, value));
        }
    }

    #[test]
    fn irq_entry_uses_seven_exact_bus_cycles() {
        let mut cpu = Cpu6510::new();
        cpu.restore_state(Cpu6510State {
            accumulator: 0,
            index_x: 0,
            index_y: 0,
            program_counter: 0x1234,
            stack_pointer: 0xfd,
            status: CPU_STATUS_BREAK | CPU_STATUS_CARRY,
        });
        let mut bus = TestBus::new();
        bus.bytes[0xfffe] = 0x78;
        bus.bytes[0xffff] = 0x56;
        assert!(cpu.begin_maskable_interrupt_sequence());
        for cycle in 0..7 {
            assert_eq!(cpu.clock_cycle(&mut bus).unwrap(), cycle == 6);
        }
        assert_eq!(
            bus.accesses,
            [
                (Kind::Read, 0x1234, 0x00),
                (Kind::Read, 0x1234, 0x00),
                (Kind::Write, 0x01fd, 0x12),
                (Kind::Write, 0x01fc, 0x34),
                (Kind::Write, 0x01fb, CPU_STATUS_CARRY | CPU_STATUS_UNUSED),
                (Kind::Read, 0xfffe, 0x78),
                (Kind::Read, 0xffff, 0x56),
            ]
        );
        assert_eq!(cpu.state().program_counter, 0x5678);
        assert_eq!(cpu.state().stack_pointer, 0xfa);
        assert_ne!(cpu.state().status & CPU_STATUS_INTERRUPT_DISABLE, 0);
        assert!(cpu.is_at_instruction_boundary());
    }

    #[test]
    fn mature_nmi_takes_over_an_irq_at_the_vector_selection_cycle() {
        let mut cpu = Cpu6510::new();
        cpu.restore_state(Cpu6510State {
            program_counter: 0x2000,
            ..Cpu6510State::deterministic_power_on()
        });
        let mut bus = TestBus::new();
        bus.bytes[0xfffa] = 0xcd;
        bus.bytes[0xfffb] = 0xab;
        bus.bytes[0xfffe] = 0x34;
        bus.bytes[0xffff] = 0x12;
        assert!(cpu.begin_maskable_interrupt_sequence());
        for _ in 0..5 {
            assert!(!cpu.clock_cycle(&mut bus).unwrap());
        }
        cpu.request_nmi_takeover();
        assert!(!cpu.clock_cycle(&mut bus).unwrap());
        assert!(cpu.clock_cycle(&mut bus).unwrap());
        assert_eq!(cpu.state().program_counter, 0xabcd);
        assert_eq!(bus.accesses[5].1, 0xfffa);
        assert_eq!(bus.accesses[6].1, 0xfffb);
    }
}
