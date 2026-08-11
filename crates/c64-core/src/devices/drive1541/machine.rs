// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 independent 6502 scheduler
//
//   File:       devices/drive1541/machine.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::cpu::{Cpu6510, Cpu6510Error, CpuBus, CpuIrqLine};
use crate::devices::iec::IecBus;

use super::mechanism::{Drive1541Mechanism, Drive1541MechanismError};
use super::memory::{Drive1541Memory, Drive1541MemoryError};

const CPU_BOUNDARY_CLOCK_OFFSET: u64 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Drive1541MachineError {
    ClockOverflow,
    Cpu(Cpu6510Error),
    Mechanism(Drive1541MechanismError),
    Memory(Drive1541MemoryError),
}

impl fmt::Display for Drive1541MachineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClockOverflow => write!(formatter, "1541 elapsed cycle counter overflowed"),
            Self::Cpu(error) => error.fmt(formatter),
            Self::Mechanism(error) => error.fmt(formatter),
            Self::Memory(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Drive1541MachineError {}

impl From<Cpu6510Error> for Drive1541MachineError {
    fn from(error: Cpu6510Error) -> Self {
        Self::Cpu(error)
    }
}

impl From<Drive1541MechanismError> for Drive1541MachineError {
    fn from(error: Drive1541MechanismError) -> Self {
        Self::Mechanism(error)
    }
}

impl From<Drive1541MemoryError> for Drive1541MachineError {
    fn from(error: Drive1541MemoryError) -> Self {
        Self::Memory(error)
    }
}

/// Owns the drive CPU, decoder and mechanism in one monotonic 1 MHz domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Drive1541Machine {
    cpu: Cpu6510,
    memory: Drive1541Memory,
    mechanism: Drive1541Mechanism,
    irq_line: CpuIrqLine,
    elapsed_cycles: u64,
    observed_byte_ready_edge_sequence: u64,
}

impl Drive1541Machine {
    /// Construct a drive CPU and perform its initial reset-vector reads without
    /// advancing the externally visible machine clock.
    ///
    /// # Errors
    ///
    /// Propagates memory errors encountered while reading the reset vector.
    pub fn new(
        mut memory: Drive1541Memory,
        mut mechanism: Drive1541Mechanism,
        iec_bus: &mut IecBus,
    ) -> Result<Self, Drive1541MachineError> {
        let mut cpu = Cpu6510::new();
        let reset_vector_low = memory.read(iec_bus, &mut mechanism, 0xfffc)?;
        let reset_vector_high = memory.read(iec_bus, &mut mechanism, 0xfffd)?;
        let mut cpu_state = cpu.state();
        cpu_state.program_counter = u16::from_le_bytes([reset_vector_low, reset_vector_high]);
        cpu.restore_state(cpu_state);
        let observed_byte_ready_edge_sequence = mechanism.byte_ready_edge_sequence();
        Ok(Self {
            cpu,
            memory,
            mechanism,
            irq_line: CpuIrqLine::new(),
            elapsed_cycles: 0,
            observed_byte_ready_edge_sequence,
        })
    }

    pub const fn cpu(&self) -> &Cpu6510 {
        &self.cpu
    }

    pub const fn memory(&self) -> &Drive1541Memory {
        &self.memory
    }

    pub const fn mechanism(&self) -> &Drive1541Mechanism {
        &self.mechanism
    }

    pub const fn mechanism_mut(&mut self) -> &mut Drive1541Mechanism {
        &mut self.mechanism
    }

    pub const fn elapsed_cycles(&self) -> u64 {
        self.elapsed_cycles
    }

    pub const fn disk_via_interrupt_pending(&self) -> bool {
        self.memory.disk_via().interrupt_pending()
    }

    /// Perform one CPU bus cycle with one preceding hardware cycle.
    ///
    /// # Errors
    ///
    /// Propagates CPU table, mechanism, decoder and clock-overflow errors.
    pub fn clock_cycle(&mut self, iec_bus: &mut IecBus) -> Result<bool, Drive1541MachineError> {
        self.service_pending_interrupt();
        if self.advance_hardware_one_cycle(iec_bus)? {
            self.cpu.signal_set_overflow();
        }
        let (cpu_result, memory_error) = {
            let mut cpu_bus =
                Drive1541CpuBus::passive(&mut self.memory, &mut self.mechanism, iec_bus);
            let result = self.cpu.clock_cycle(&mut cpu_bus);
            (result, cpu_bus.take_error())
        };
        self.synchronize_interrupt_input();
        if let Some(error) = memory_error {
            return Err(error);
        }
        Ok(cpu_result?)
    }

    /// Perform an exact number of adjacent CPU bus cycles.
    ///
    /// # Errors
    ///
    /// Propagates the first cycle error.
    pub fn clock_cycles(
        &mut self,
        iec_bus: &mut IecBus,
        cycles: u64,
    ) -> Result<u64, Drive1541MachineError> {
        for _ in 0..cycles {
            self.clock_cycle(iec_bus)?;
        }
        Ok(cycles)
    }

    /// Execute through the next instruction boundary or JAM state.
    ///
    /// # Errors
    ///
    /// Propagates the first CPU or hardware error.
    pub fn execute_instruction(
        &mut self,
        iec_bus: &mut IecBus,
    ) -> Result<u64, Drive1541MachineError> {
        let start_cycle = self.elapsed_cycles;
        loop {
            self.clock_cycle(iec_bus)?;
            if self.cpu.is_at_instruction_boundary() || self.cpu.is_jammed() {
                return Ok(self.elapsed_cycles - start_cycle);
            }
        }
    }

    /// Advance peripherals without executing the drive CPU.
    ///
    /// # Errors
    ///
    /// Propagates the first mechanism, decoder or clock-overflow error.
    pub fn advance_hardware(
        &mut self,
        iec_bus: &mut IecBus,
        cycles: u64,
    ) -> Result<u64, Drive1541MachineError> {
        for _ in 0..cycles {
            if self.advance_hardware_one_cycle(iec_bus)? {
                self.cpu.signal_set_overflow();
            }
        }
        Ok(cycles)
    }

    /// Execute seven interleaved hardware/reset bus cycles.
    ///
    /// # Errors
    ///
    /// Propagates the first mechanism, decoder or clock-overflow error.
    pub fn reset_cpu(&mut self, iec_bus: &mut IecBus) -> Result<u64, Drive1541MachineError> {
        self.irq_line.reset();
        let start_cycle = self.elapsed_cycles;
        let (memory_error, set_overflow) = {
            let timing = Drive1541BusTiming {
                elapsed_cycles: &mut self.elapsed_cycles,
                irq_line: &mut self.irq_line,
                observed_byte_ready_edge_sequence: &mut self.observed_byte_ready_edge_sequence,
            };
            let mut cpu_bus =
                Drive1541CpuBus::clocked(&mut self.memory, &mut self.mechanism, iec_bus, timing);
            self.cpu.reset(&mut cpu_bus);
            (cpu_bus.take_error(), cpu_bus.set_overflow_seen())
        };
        if set_overflow {
            self.cpu.signal_set_overflow();
        }
        self.synchronize_interrupt_input();
        if let Some(error) = memory_error {
            return Err(error);
        }
        Ok(self.elapsed_cycles - start_cycle)
    }

    /// Reset only the scheduler clock and IRQ latch.
    pub fn reset_timing(&mut self) {
        self.irq_line.reset();
        self.elapsed_cycles = 0;
        self.observed_byte_ready_edge_sequence = self.mechanism.byte_ready_edge_sequence();
    }

    /// Perform a debugger-style memory read without advancing time.
    ///
    /// # Errors
    ///
    /// Propagates decoder errors.
    pub fn read_memory(
        &mut self,
        iec_bus: &mut IecBus,
        address: u16,
    ) -> Result<u8, Drive1541MachineError> {
        let value = self.memory.read(iec_bus, &mut self.mechanism, address)?;
        self.synchronize_interrupt_input();
        Ok(value)
    }

    /// Perform a debugger-style memory write without advancing time.
    ///
    /// # Errors
    ///
    /// Propagates decoder errors.
    pub fn write_memory(
        &mut self,
        iec_bus: &mut IecBus,
        address: u16,
        value: u8,
    ) -> Result<(), Drive1541MachineError> {
        self.memory
            .write(iec_bus, &mut self.mechanism, address, value)?;
        self.synchronize_interrupt_input();
        Ok(())
    }

    fn advance_hardware_one_cycle(
        &mut self,
        iec_bus: &mut IecBus,
    ) -> Result<bool, Drive1541MachineError> {
        advance_hardware_fields(
            &mut self.memory,
            &mut self.mechanism,
            iec_bus,
            &mut self.elapsed_cycles,
            &mut self.irq_line,
            &mut self.observed_byte_ready_edge_sequence,
        )
    }

    fn service_pending_interrupt(&mut self) {
        if !self.cpu.is_at_instruction_boundary() {
            return;
        }
        self.synchronize_interrupt_input();
        let boundary_clock = self
            .elapsed_cycles
            .saturating_add(CPU_BOUNDARY_CLOCK_OFFSET);
        if self.irq_line.is_pending(boundary_clock)
            && self
                .cpu
                .can_accept_maskable_interrupt(self.irq_line.asserted_cycles(boundary_clock))
        {
            self.irq_line.acknowledge();
            self.cpu.begin_maskable_interrupt_sequence();
        }
        self.irq_line.complete_cpu_boundary_poll();
    }

    fn synchronize_interrupt_input(&mut self) {
        self.irq_line.update(
            self.memory.iec_via().interrupt_pending() || self.memory.disk_via().interrupt_pending(),
            self.elapsed_cycles,
        );
    }
}

struct Drive1541BusTiming<'a> {
    elapsed_cycles: &'a mut u64,
    irq_line: &'a mut CpuIrqLine,
    observed_byte_ready_edge_sequence: &'a mut u64,
}

struct Drive1541CpuBus<'a> {
    memory: &'a mut Drive1541Memory,
    mechanism: &'a mut Drive1541Mechanism,
    iec_bus: &'a mut IecBus,
    timing: Option<Drive1541BusTiming<'a>>,
    error: Option<Drive1541MachineError>,
    set_overflow_seen: bool,
}

impl<'a> Drive1541CpuBus<'a> {
    fn passive(
        memory: &'a mut Drive1541Memory,
        mechanism: &'a mut Drive1541Mechanism,
        iec_bus: &'a mut IecBus,
    ) -> Self {
        Self {
            memory,
            mechanism,
            iec_bus,
            timing: None,
            error: None,
            set_overflow_seen: false,
        }
    }

    fn clocked(
        memory: &'a mut Drive1541Memory,
        mechanism: &'a mut Drive1541Mechanism,
        iec_bus: &'a mut IecBus,
        timing: Drive1541BusTiming<'a>,
    ) -> Self {
        Self {
            memory,
            mechanism,
            iec_bus,
            timing: Some(timing),
            error: None,
            set_overflow_seen: false,
        }
    }

    fn take_error(&mut self) -> Option<Drive1541MachineError> {
        self.error.take()
    }

    const fn set_overflow_seen(&self) -> bool {
        self.set_overflow_seen
    }

    fn begin_bus_cycle(&mut self) {
        if self.error.is_some() {
            return;
        }
        let Some(timing) = self.timing.as_mut() else {
            return;
        };
        match advance_hardware_fields(
            self.memory,
            self.mechanism,
            self.iec_bus,
            timing.elapsed_cycles,
            timing.irq_line,
            timing.observed_byte_ready_edge_sequence,
        ) {
            Ok(set_overflow) => self.set_overflow_seen |= set_overflow,
            Err(error) => self.error = Some(error),
        }
    }

    fn record_memory_error(&mut self, error: Drive1541MemoryError) {
        if self.error.is_none() {
            self.error = Some(error.into());
        }
    }
}

impl CpuBus for Drive1541CpuBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        self.begin_bus_cycle();
        if self.error.is_some() {
            return self.memory.last_data_bus_value();
        }
        match self.memory.read(self.iec_bus, self.mechanism, address) {
            Ok(value) => value,
            Err(error) => {
                self.record_memory_error(error);
                self.memory.last_data_bus_value()
            }
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        self.begin_bus_cycle();
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self
            .memory
            .write(self.iec_bus, self.mechanism, address, value)
        {
            self.record_memory_error(error);
        }
    }
}

fn advance_hardware_fields(
    memory: &mut Drive1541Memory,
    mechanism: &mut Drive1541Mechanism,
    iec_bus: &mut IecBus,
    elapsed_cycles: &mut u64,
    irq_line: &mut CpuIrqLine,
    observed_byte_ready_edge_sequence: &mut u64,
) -> Result<bool, Drive1541MachineError> {
    let next_cycle = elapsed_cycles
        .checked_add(1)
        .ok_or(Drive1541MachineError::ClockOverflow)?;
    mechanism.tick(1)?;
    memory.clock_peripherals(iec_bus, mechanism)?;
    *elapsed_cycles = next_cycle;
    irq_line.update(
        memory.iec_via().interrupt_pending() || memory.disk_via().interrupt_pending(),
        next_cycle,
    );
    let edge_sequence = mechanism.byte_ready_edge_sequence();
    let set_overflow = edge_sequence != *observed_byte_ready_edge_sequence;
    *observed_byte_ready_edge_sequence = edge_sequence;
    Ok(set_overflow)
}
