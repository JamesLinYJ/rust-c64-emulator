// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 整机核心所有权与粗粒度调度
//
//   文件:       machine.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::{
    address_space::{C64AddressSpace, C64BusDevices, C64Firmware},
    architecture::{
        CoreConfig, ExecutionConfigError, ExecutionController, ExecutionRequest, ExecutionStatus,
        MachineProfile, SlotsPerSystemCycle, TurboSpeedRequest,
    },
    clock::{VirtualClock, VirtualClockError, VirtualTimestamp},
    cpu::{Cpu6510, Cpu6510Error, Cpu6510State, CpuBus, CpuIrqLine, CpuNmiLine},
    devices::{C64Chipset, C64ChipsetError, vic::VicError},
    memory::{CoherentMemory, MemoryWriteSource},
    state::{self, StateError, StateImage},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuBusAccessKind {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuBusTransaction {
    pub kind: CpuBusAccessKind,
    pub address: u16,
    pub value: u8,
    pub timestamp: VirtualTimestamp,
    pub read_was_held: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CoreDiagnostics {
    pub retired_cpu_slots: u64,
    pub elapsed_system_cycles: u64,
    pub held_cpu_read_system_cycles: u64,
    pub cpu_bus_transactions: u64,
    pub last_cpu_bus_transaction: Option<CpuBusTransaction>,
    pub execution_mode_changes: u64,
    pub state_loads: u64,
}

impl CoreDiagnostics {
    fn record_cpu_bus_transaction(&mut self, transaction: CpuBusTransaction) {
        self.cpu_bus_transactions = self.cpu_bus_transactions.wrapping_add(1);
        self.last_cpu_bus_transaction = Some(transaction);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C64Core {
    config: CoreConfig,
    execution: ExecutionController,
    clock: VirtualClock,
    cpu: Cpu6510,
    address_space: C64AddressSpace,
    devices: C64Chipset,
    irq_line: CpuIrqLine,
    nmi_line: CpuNmiLine,
    diagnostics: CoreDiagnostics,
}

impl Default for C64Core {
    fn default() -> Self {
        Self::new(CoreConfig::default())
    }
}

impl C64Core {
    pub fn new(config: CoreConfig) -> Self {
        Self::with_firmware(config, C64Firmware::blank_for_test())
    }

    pub fn with_firmware(config: CoreConfig, firmware: C64Firmware) -> Self {
        let devices = C64Chipset::new_with_sid_model(config.video_standard, config.sid_model);
        Self {
            config,
            execution: ExecutionController::new(),
            clock: VirtualClock::new(),
            cpu: Cpu6510::new(),
            address_space: C64AddressSpace::new(firmware),
            devices,
            irq_line: CpuIrqLine::new(),
            nmi_line: CpuNmiLine::new(),
            diagnostics: CoreDiagnostics::default(),
        }
    }

    pub const fn config(&self) -> CoreConfig {
        self.config
    }

    pub const fn execution_status(&self) -> ExecutionStatus {
        self.execution.status()
    }

    pub const fn timestamp(&self) -> VirtualTimestamp {
        self.clock.timestamp()
    }

    pub const fn cpu_state(&self) -> Cpu6510State {
        self.cpu.state()
    }

    pub const fn cpu_is_at_instruction_boundary(&self) -> bool {
        self.cpu.is_at_instruction_boundary()
    }

    pub const fn cpu_is_jammed(&self) -> bool {
        self.cpu.is_jammed()
    }

    /// 在已提交指令边界修改下一条 CPU 取指地址；不会中断正在进行的总线微序列。
    pub fn set_cpu_program_counter(&mut self, program_counter: u16) -> bool {
        self.cpu
            .set_program_counter_at_instruction_boundary(program_counter)
    }

    pub const fn diagnostics(&self) -> CoreDiagnostics {
        self.diagnostics
    }

    pub const fn memory(&self) -> &CoherentMemory {
        self.address_space.memory()
    }

    pub const fn devices(&self) -> &C64Chipset {
        &self.devices
    }

    /// Attach one explicitly configured 1541 at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot, duplicate attachment, invalid drive
    /// configuration or a full IEC bus.
    pub fn attach_drive1541(&mut self, device_number: u8, rom: &[u8]) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.attach_drive1541(device_number, rom)?;
        Ok(())
    }

    /// Detach the configured 1541 at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot or an empty drive slot and propagates IEC
    /// detach errors.
    pub fn detach_drive1541(&mut self) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.detach_drive1541()?;
        Ok(())
    }

    /// Borrow the attached drive mutably at a system-cycle boundary for coarse
    /// media and debugger operations.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot or an empty drive slot.
    pub fn drive1541_mut(
        &mut self,
    ) -> Result<&mut crate::devices::drive1541::drive::Commodore1541Drive, CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices
            .drive1541_mut()
            .ok_or(C64ChipsetError::Drive1541NotAttached.into())
    }

    /// 在下一个公开提交边界应用执行请求。
    ///
    /// # Errors
    ///
    /// 当前不在系统周期边界时返回时钟错误。
    pub fn request_execution(&mut self, request: ExecutionRequest) -> Result<(), CoreError> {
        let mut next = self.execution;
        next.request(request);
        self.clock
            .set_slots_per_system_cycle(next.status().effective_slots)?;
        self.execution = next;
        self.diagnostics.execution_mode_changes =
            self.diagnostics.execution_mode_changes.wrapping_add(1);
        Ok(())
    }

    /// 请求一个经过范围校验的手动 Turbo 档位。
    ///
    /// # Errors
    ///
    /// 槽位超出 2..=64 或当前不在边界时返回错误。
    pub fn request_manual_turbo(&mut self, slots: u8) -> Result<(), CoreError> {
        self.request_execution(ExecutionRequest::Turbo(TurboSpeedRequest::manual(slots)?))
    }

    /// 请求一次性校准、随后锁定的 Auto Turbo。
    ///
    /// # Errors
    ///
    /// 最大槽位超出范围或当前不在边界时返回错误。
    pub fn request_auto_turbo(&mut self, maximum: u8) -> Result<(), CoreError> {
        self.request_execution(ExecutionRequest::Turbo(TurboSpeedRequest::automatic(
            maximum,
        )?))
    }

    /// 将待校准 Auto 请求锁定为一个确定档位。
    ///
    /// # Errors
    ///
    /// 未请求 Auto、档位超过上限或当前不在边界时返回错误。
    pub fn lock_auto_turbo(&mut self, resolved: u8) -> Result<(), CoreError> {
        let resolved = if resolved == SlotsPerSystemCycle::STRICT.get() {
            SlotsPerSystemCycle::STRICT
        } else {
            SlotsPerSystemCycle::try_turbo(resolved)?
        };
        let mut next = self.execution;
        next.lock_auto_slots(resolved)?;
        self.clock.set_slots_per_system_cycle(resolved)?;
        self.execution = next;
        self.diagnostics.execution_mode_changes =
            self.diagnostics.execution_mode_changes.wrapping_add(1);
        Ok(())
    }

    /// 执行硬件 Reset 的架构部分并恢复 Strict/1MHz。
    ///
    /// # Errors
    ///
    /// 当前不在合法提交边界时返回时钟错误。
    pub fn reset(&mut self) -> Result<(), CoreError> {
        self.clock
            .set_slots_per_system_cycle(SlotsPerSystemCycle::STRICT)?;
        self.execution.reset_to_strict();
        self.devices.reset()?;
        self.irq_line.reset();
        self.nmi_line.reset();
        self.address_space
            .reset_processor_port(self.devices.cartridge_lines());
        self.devices
            .processor_port_output_changed(self.address_space.processor_port().output_state());
        self.cpu
            .restore_state(Cpu6510State::deterministic_power_on());
        {
            let mut bus = ClockedCpuBus::new(
                &mut self.address_space,
                &mut self.devices,
                &mut self.clock,
                &mut self.irq_line,
                &mut self.nmi_line,
                &mut self.diagnostics,
            );
            self.cpu.reset(&mut bus);
            if let Some(error) = bus.take_hardware_error() {
                return Err(error.into());
            }
        }
        self.diagnostics.execution_mode_changes =
            self.diagnostics.execution_mode_changes.wrapping_add(1);
        Ok(())
    }

    /// Power-cycle the C64 board while retaining explicitly attached external
    /// drive configuration and physical media.
    ///
    /// # Errors
    ///
    /// Propagates a 1541 reset error without committing the new machine state.
    pub fn power_cycle_with_profile(&mut self, profile: MachineProfile) -> Result<(), CoreError> {
        let mut devices = self.devices.clone();
        devices.reinitialize_host(self.config.video_standard, self.config.sid_model)?;
        let address_space = C64AddressSpace::new(self.address_space.firmware().clone());
        self.config.profile = profile;
        self.execution.reset_to_strict();
        self.clock.reset();
        self.cpu = Cpu6510::new();
        self.address_space = address_space;
        self.devices = devices;
        self.irq_line.reset();
        self.nmi_line.reset();
        self.diagnostics = CoreDiagnostics::default();
        Ok(())
    }

    /// 从系统周期边界推进 legacy 时钟域。
    ///
    /// # Errors
    ///
    /// 当前位于内部槽位时返回时钟错误。
    pub fn run_system_cycles(&mut self, cycles: u64) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        let slots_per_cycle = self.execution.status().effective_slots.get();
        for _ in 0..cycles {
            self.run_cpu_slots(u64::from(slots_per_cycle))?;
        }
        Ok(())
    }

    /// 推进指定数量的内部 CPU 槽位。
    ///
    /// # Errors
    ///
    /// 生成的 CPU 架构表与严格执行器不一致时返回内部 CPU 错误。
    pub fn run_cpu_slots(&mut self, slots: u64) -> Result<u64, CoreError> {
        let mut elapsed_cycles = 0;
        for _ in 0..slots {
            self.service_pending_interrupt();
            self.service_nmi_vector_takeover();
            let (completed_system_cycle, hardware_error) = {
                let mut bus = ClockedCpuBus::new(
                    &mut self.address_space,
                    &mut self.devices,
                    &mut self.clock,
                    &mut self.irq_line,
                    &mut self.nmi_line,
                    &mut self.diagnostics,
                );
                let cpu_result = self.cpu.clock_cycle(&mut bus);
                let completed = bus.completed_system_cycle();
                let hardware_error = bus.take_hardware_error();
                cpu_result?;
                (completed, hardware_error)
            };
            if let Some(error) = hardware_error {
                return Err(error.into());
            }
            if completed_system_cycle {
                elapsed_cycles += 1;
            }
        }
        Ok(elapsed_cycles)
    }

    pub fn read_base_ram(&self, address: u16) -> u8 {
        self.address_space.read_base_ram(address)
    }

    pub fn write_base_ram(&mut self, address: u16, value: u8, source: MemoryWriteSource) {
        self.address_space.write_base_ram(address, value, source);
    }

    pub fn save_state(&self) -> Vec<u8> {
        state::encode(&StateImage {
            config: self.config,
            execution: self.execution,
            timestamp: self.clock.timestamp(),
            cpu: self.cpu.state(),
            base_ram: self.address_space.clone_base_ram(),
        })
    }

    /// 原子载入一个经过完整验证的版本化架构状态。
    ///
    /// # Errors
    ///
    /// 头部、版本、section、配置、时钟状态或外设复位无效时返回错误。
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), CoreError> {
        let image = state::decode(bytes)?;
        let mut address_space = C64AddressSpace::new(self.address_space.firmware().clone());
        address_space.restore_base_ram(image.base_ram.as_ref());
        let mut devices = self.devices.clone();
        devices.reinitialize_host(image.config.video_standard, image.config.sid_model)?;
        self.config = image.config;
        self.execution = image.execution;
        self.clock
            .restore(image.timestamp, image.execution.status().effective_slots);
        self.cpu.restore_state(image.cpu);
        self.address_space = address_space;
        self.devices = devices;
        self.irq_line.reset();
        self.nmi_line.reset();
        self.diagnostics.state_loads = self.diagnostics.state_loads.wrapping_add(1);
        Ok(())
    }

    fn service_pending_interrupt(&mut self) {
        if !self.cpu.is_at_instruction_boundary() {
            return;
        }
        let boundary_clock = self.clock.timestamp().system_cycle.saturating_add(1);
        if self.nmi_line.is_pending()
            && self
                .cpu
                .can_accept_non_maskable_interrupt(self.nmi_line.elapsed_cycles(boundary_clock))
        {
            self.nmi_line.acknowledge();
            self.irq_line.complete_cpu_boundary_poll();
            self.cpu.begin_non_maskable_interrupt_sequence();
            return;
        }

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

    fn service_nmi_vector_takeover(&mut self) {
        if !self.cpu.interrupt_vector_selection_pending() || !self.nmi_line.is_pending() {
            return;
        }
        let vector_clock = self.clock.timestamp().system_cycle.saturating_add(1);
        if self
            .cpu
            .can_take_over_interrupt_sequence_with_nmi(self.nmi_line.elapsed_cycles(vector_clock))
        {
            self.nmi_line.acknowledge();
            self.cpu.request_nmi_takeover();
        }
    }
}

struct ClockedCpuBus<'a> {
    address_space: &'a mut C64AddressSpace,
    devices: &'a mut C64Chipset,
    clock: &'a mut VirtualClock,
    irq_line: &'a mut CpuIrqLine,
    nmi_line: &'a mut CpuNmiLine,
    diagnostics: &'a mut CoreDiagnostics,
    completed_system_cycle: bool,
    hardware_error: Option<C64ChipsetError>,
}

impl<'a> ClockedCpuBus<'a> {
    fn new(
        address_space: &'a mut C64AddressSpace,
        devices: &'a mut C64Chipset,
        clock: &'a mut VirtualClock,
        irq_line: &'a mut CpuIrqLine,
        nmi_line: &'a mut CpuNmiLine,
        diagnostics: &'a mut CoreDiagnostics,
    ) -> Self {
        Self {
            address_space,
            devices,
            clock,
            irq_line,
            nmi_line,
            diagnostics,
            completed_system_cycle: false,
            hardware_error: None,
        }
    }

    const fn completed_system_cycle(&self) -> bool {
        self.completed_system_cycle
    }

    fn take_hardware_error(&mut self) -> Option<C64ChipsetError> {
        self.hardware_error.take()
    }

    fn begin_cpu_slot(&mut self, cpu_read_address: Option<u16>) {
        if self.clock.timestamp().slot != 0 {
            return;
        }
        self.clock_hardware_cycle(cpu_read_address);
    }

    fn clock_hardware_cycle(&mut self, cpu_read_address: Option<u16>) {
        self.address_space.tick_processor_port(1);
        self.devices.clock_cias();
        if let Some(address) = cpu_read_address {
            self.address_space
                .drive_passive_cpu_read_data_bus(address, self.devices.cartridge_lines());
        }
        let vic_bank_address = self.devices.vic_bank_address();
        let result = {
            let mut vic_memory = self.address_space.vic_memory_bus(vic_bank_address);
            self.devices.clock_vic(&mut vic_memory)
        };
        if let Err(error) = result
            && self.hardware_error.is_none()
        {
            self.hardware_error = Some(error.into());
        }
        self.devices.clock_sid();
        let result = self.devices.clock_drive1541();
        if let Err(error) = result
            && self.hardware_error.is_none()
        {
            self.hardware_error = Some(error);
        }
        let sampled_cycle = self.clock.timestamp().system_cycle.saturating_add(1);
        self.irq_line
            .update(self.devices.irq_asserted(), sampled_cycle);
        self.nmi_line
            .update(self.devices.nmi_asserted(), sampled_cycle);
    }

    fn finish_held_system_cycle(&mut self, cpu_read_address: u16) {
        self.clock.advance_external_wait_cycle();
        self.diagnostics.elapsed_system_cycles =
            self.diagnostics.elapsed_system_cycles.wrapping_add(1);
        self.clock_hardware_cycle(Some(cpu_read_address));
    }

    fn finish_cpu_slot(&mut self) {
        self.completed_system_cycle = self.clock.consume_cpu_slot();
        self.diagnostics.retired_cpu_slots = self.diagnostics.retired_cpu_slots.wrapping_add(1);
        if self.completed_system_cycle {
            self.diagnostics.elapsed_system_cycles =
                self.diagnostics.elapsed_system_cycles.wrapping_add(1);
        }
    }
}

impl CpuBus for ClockedCpuBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        self.devices.begin_cpu_read();
        self.begin_cpu_slot(Some(address));
        while self.devices.ba_low() && self.hardware_error.is_none() {
            self.devices.mark_cpu_read_held();
            self.diagnostics.held_cpu_read_system_cycles =
                self.diagnostics.held_cpu_read_system_cycles.wrapping_add(1);
            self.finish_held_system_cycle(address);
        }
        let value = {
            let mut bus = self.address_space.cpu_bus(self.devices);
            bus.read(address)
        };
        self.diagnostics
            .record_cpu_bus_transaction(CpuBusTransaction {
                kind: CpuBusAccessKind::Read,
                address,
                value,
                timestamp: self.clock.timestamp(),
                read_was_held: self.devices.cpu_read_was_held(),
            });
        self.finish_cpu_slot();
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        self.begin_cpu_slot(None);
        {
            let mut bus = self.address_space.cpu_bus(self.devices);
            bus.write(address, value);
        }
        self.diagnostics
            .record_cpu_bus_transaction(CpuBusTransaction {
                kind: CpuBusAccessKind::Write,
                address,
                value,
                timestamp: self.clock.timestamp(),
                read_was_held: false,
            });
        self.finish_cpu_slot();
    }

    fn read_was_held(&self) -> bool {
        self.devices.cpu_read_was_held()
    }
}

#[derive(Debug)]
pub enum CoreError {
    Chipset(C64ChipsetError),
    Execution(ExecutionConfigError),
    Clock(VirtualClockError),
    Cpu(Cpu6510Error),
    State(StateError),
    Vic(VicError),
}

impl From<ExecutionConfigError> for CoreError {
    fn from(error: ExecutionConfigError) -> Self {
        Self::Execution(error)
    }
}

impl From<C64ChipsetError> for CoreError {
    fn from(error: C64ChipsetError) -> Self {
        match error {
            C64ChipsetError::Vic(error) => Self::Vic(error),
            other => Self::Chipset(other),
        }
    }
}

impl From<VirtualClockError> for CoreError {
    fn from(error: VirtualClockError) -> Self {
        Self::Clock(error)
    }
}

impl From<StateError> for CoreError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<Cpu6510Error> for CoreError {
    fn from(error: Cpu6510Error) -> Self {
        Self::Cpu(error)
    }
}

impl From<VicError> for CoreError {
    fn from(error: VicError) -> Self {
        Self::Vic(error)
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chipset(error) => error.fmt(formatter),
            Self::Execution(error) => error.fmt(formatter),
            Self::Clock(error) => error.fmt(formatter),
            Self::Cpu(error) => error.fmt(formatter),
            Self::State(error) => error.fmt(formatter),
            Self::Vic(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use crate::{
        address_space::{BASIC_ROM_BYTES, C64Firmware, CHARACTER_ROM_BYTES, KERNAL_ROM_BYTES},
        architecture::{CoreConfig, MachineProfile},
        memory::MemoryWriteSource,
    };

    use super::{C64Core, CpuBusAccessKind};

    #[test]
    fn debugger_pc_changes_require_a_boundary_and_bus_diagnostics_are_generic() {
        let mut core = C64Core::default();
        core.write_base_ram(0x1234, 0xea, MemoryWriteSource::HostLoader);
        assert!(core.set_cpu_program_counter(0x1234));
        let transactions_before = core.diagnostics().cpu_bus_transactions;

        core.run_cpu_slots(1).unwrap();
        let diagnostics = core.diagnostics();
        assert_eq!(diagnostics.cpu_bus_transactions, transactions_before + 1);
        assert_eq!(
            diagnostics.last_cpu_bus_transaction.map(|transaction| (
                transaction.kind,
                transaction.address,
                transaction.value,
                transaction.read_was_held,
            )),
            Some((CpuBusAccessKind::Read, 0x1234, 0xea, false))
        );
        assert!(!core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(1).unwrap();
        assert!(core.cpu_is_at_instruction_boundary());
        assert!(core.set_cpu_program_counter(0x2000));
    }

    #[test]
    fn reset_preserves_profile_but_returns_to_strict() {
        let mut core = C64Core::new(CoreConfig {
            profile: MachineProfile::SuperCpu,
            ..CoreConfig::default()
        });
        core.request_manual_turbo(20).unwrap();
        core.reset().unwrap();
        assert_eq!(core.config().profile, MachineProfile::SuperCpu);
        assert!(!core.execution_status().is_turbo());
    }

    #[test]
    fn save_state_round_trip_restores_architectural_configuration_and_ram() {
        let mut source = C64Core::default();
        source.request_auto_turbo(48).unwrap();
        source.lock_auto_turbo(24).unwrap();
        source.run_system_cycles(12_345).unwrap();
        source.write_base_ram(0xc000, 0x5a, MemoryWriteSource::EnhancedDma);
        let state = source.save_state();

        let mut restored = C64Core::default();
        restored.load_state(&state).unwrap();
        assert_eq!(restored.config(), source.config());
        assert_eq!(restored.execution_status(), source.execution_status());
        assert_eq!(restored.timestamp(), source.timestamp());
        assert_eq!(restored.cpu_state(), source.cpu_state());
        assert_eq!(restored.read_base_ram(0xc000), 0x5a);
        assert_eq!(restored.diagnostics().state_loads, 1);
    }

    #[test]
    fn cia1_interrupt_reaches_the_cpu_through_the_cycle_scheduler() {
        let basic = [0_u8; BASIC_ROM_BYTES];
        let character = [0_u8; CHARACTER_ROM_BYTES];
        let mut kernal = [0_u8; KERNAL_ROM_BYTES];
        kernal[0x1ffc..=0x1ffd].copy_from_slice(&0x1000_u16.to_le_bytes());
        kernal[0x1ffe..=0x1fff].copy_from_slice(&0x2000_u16.to_le_bytes());
        let firmware = C64Firmware::new(&basic, &character, &kernal).unwrap();
        let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);

        // Configure CIA1 Timer A, enable its IRQ, then unmask the 6510.
        for (address, value) in [
            (0x1000, 0xa9),
            (0x1001, 0x01),
            (0x1002, 0x8d),
            (0x1003, 0x04),
            (0x1004, 0xdc),
            (0x1005, 0xa9),
            (0x1006, 0x00),
            (0x1007, 0x8d),
            (0x1008, 0x05),
            (0x1009, 0xdc),
            (0x100a, 0xa9),
            (0x100b, 0x81),
            (0x100c, 0x8d),
            (0x100d, 0x0d),
            (0x100e, 0xdc),
            (0x100f, 0xa9),
            (0x1010, 0x11),
            (0x1011, 0x8d),
            (0x1012, 0x0e),
            (0x1013, 0xdc),
            (0x1014, 0x58),
            (0x1015, 0xea),
            (0x1016, 0x4c),
            (0x1017, 0x15),
            (0x1018, 0x10),
            // IRQ: INC $c000; LDA $dc0d; RTI
            (0x2000, 0xee),
            (0x2001, 0x00),
            (0x2002, 0xc0),
            (0x2003, 0xad),
            (0x2004, 0x0d),
            (0x2005, 0xdc),
            (0x2006, 0x40),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }

        core.reset().unwrap();
        assert_eq!(core.cpu_state().program_counter, 0x1000);
        assert_eq!(core.timestamp().system_cycle, 7);
        core.run_system_cycles(200).unwrap();
        assert_ne!(core.read_base_ram(0xc000), 0);
    }

    #[test]
    fn vic_raster_interrupt_reaches_the_cpu_through_the_cycle_scheduler() {
        let basic = [0_u8; BASIC_ROM_BYTES];
        let character = [0_u8; CHARACTER_ROM_BYTES];
        let mut kernal = [0_u8; KERNAL_ROM_BYTES];
        kernal[0x1ffc..=0x1ffd].copy_from_slice(&0x1000_u16.to_le_bytes());
        kernal[0x1ffe..=0x1fff].copy_from_slice(&0x2000_u16.to_le_bytes());
        let firmware = C64Firmware::new(&basic, &character, &kernal).unwrap();
        let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);

        // 先确认并清除 reset 后第 0 行的锁存，再选择第 2 行并开放 raster IRQ。
        for (address, value) in [
            (0x1000, 0xa9),
            (0x1001, 0x01),
            (0x1002, 0x8d),
            (0x1003, 0x19),
            (0x1004, 0xd0),
            (0x1005, 0xa9),
            (0x1006, 0x02),
            (0x1007, 0x8d),
            (0x1008, 0x12),
            (0x1009, 0xd0),
            (0x100a, 0xa9),
            (0x100b, 0x01),
            (0x100c, 0x8d),
            (0x100d, 0x1a),
            (0x100e, 0xd0),
            (0x100f, 0x58),
            (0x1010, 0xea),
            (0x1011, 0x4c),
            (0x1012, 0x10),
            (0x1013, 0x10),
            // IRQ: INC $c001; LDA $d019; STA $d019; RTI
            (0x2000, 0xee),
            (0x2001, 0x01),
            (0x2002, 0xc0),
            (0x2003, 0xad),
            (0x2004, 0x19),
            (0x2005, 0xd0),
            (0x2006, 0x8d),
            (0x2007, 0x19),
            (0x2008, 0xd0),
            (0x2009, 0x40),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }

        core.reset().unwrap();
        core.run_system_cycles(400).unwrap();
        assert_ne!(core.read_base_ram(0xc001), 0);
        assert!(core.devices().vic().current_raster_line() >= 2);
    }

    #[test]
    fn vic_bad_line_extends_cpu_reads_without_retiring_extra_slots() {
        let basic = [0_u8; BASIC_ROM_BYTES];
        let character = [0_u8; CHARACTER_ROM_BYTES];
        let mut kernal = [0_u8; KERNAL_ROM_BYTES];
        kernal[0x1ffc..=0x1ffd].copy_from_slice(&0x1000_u16.to_le_bytes());
        let firmware = C64Firmware::new(&basic, &character, &kernal).unwrap();
        let mut core = C64Core::with_firmware(CoreConfig::default(), firmware);
        // Enable display with YSCROLL=0, then execute a read-heavy loop through raster line $30.
        for (address, value) in [
            (0x1000, 0xa9),
            (0x1001, 0x10),
            (0x1002, 0x8d),
            (0x1003, 0x11),
            (0x1004, 0xd0),
            (0x1005, 0xea),
            (0x1006, 0x4c),
            (0x1007, 0x05),
            (0x1008, 0x10),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }

        core.reset().unwrap();
        let retired_before = core.diagnostics().retired_cpu_slots;
        let elapsed_before = core.diagnostics().elapsed_system_cycles;
        core.run_cpu_slots(4_000).unwrap();
        let diagnostics = core.diagnostics();
        assert_eq!(diagnostics.retired_cpu_slots - retired_before, 4_000);
        assert!(diagnostics.held_cpu_read_system_cycles >= 40);
        assert_eq!(
            diagnostics.elapsed_system_cycles - elapsed_before,
            4_000 + diagnostics.held_cpu_read_system_cycles
        );
    }
}
