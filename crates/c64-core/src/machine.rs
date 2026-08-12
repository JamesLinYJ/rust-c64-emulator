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
    address_space::{C64AddressSpace, C64BusDevices, C64Firmware, CartridgeLines, CartridgeRegion},
    architecture::{
        CoreConfig, ExecutionConfigError, ExecutionController, ExecutionRequest, ExecutionStatus,
        MachineProfile, SlotsPerSystemCycle, TurboControlCommand, TurboSpeedRequest,
        vm_enhanced_turbo_index, vm_enhanced_turbo_slots,
    },
    bus::{BusAccessKind, BusBridge, BusMaster, BusRequest, BusResponse},
    clock::{VirtualClock, VirtualClockError, VirtualTimestamp},
    cpu::{Cpu6510, Cpu6510Error, Cpu6510State, CpuBus, CpuIrqLine, CpuNmiLine},
    devices::{
        C64Chipset, C64ChipsetError,
        cartridge::{Cartridge, CartridgeKind},
        drive1541::mechanism::Drive1541DiskImage,
        reu::{RamExpansionUnit, ReuDmaOperation, ReuSize},
        tape::{DatasetteError, DatasetteTape, DatasetteTransport},
        vic::VicError,
    },
    media::{
        prg::{
            BASIC_KEYBOARD_BUFFER_COUNT, BASIC_KEYBOARD_BUFFER_START, BASIC_RUN_COMMAND,
            BASIC_RUN_COMMAND_LENGTH, BASIC_TEXT_END_POINTERS, BASIC_TEXT_START_POINTERS,
            BasicPrgImage, LoadedPrg, PrgError,
        },
        tap::{TapImage, TapImageError, TapVideoStandard, WritableTapImage},
    },
    memory::{CoherentMemory, MemoryWriteSource, PageDescriptor, PageDomain, PhysicalTarget},
    state::{self, DecodedState, StateError},
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BusBridgeTransaction {
    pub request: BusRequest,
    pub domain: PageDomain,
    pub response: BusResponse,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CoreDiagnostics {
    pub retired_cpu_slots: u64,
    pub elapsed_system_cycles: u64,
    pub held_cpu_read_system_cycles: u64,
    pub reu_dma_system_cycles: u64,
    pub reu_dma_vic_stall_cycles: u64,
    pub reu_dma_bus_cycles: u64,
    pub cpu_bus_transactions: u64,
    pub last_cpu_bus_transaction: Option<CpuBusTransaction>,
    pub last_bus_bridge_transaction: Option<BusBridgeTransaction>,
    pub execution_mode_changes: u64,
    pub state_loads: u64,
}

impl CoreDiagnostics {
    fn record_cpu_bus_transaction(&mut self, transaction: CpuBusTransaction) {
        self.cpu_bus_transactions = self.cpu_bus_transactions.wrapping_add(1);
        self.last_cpu_bus_transaction = Some(transaction);
    }

    fn record_bus_bridge_transaction(&mut self, transaction: BusBridgeTransaction) {
        self.last_bus_bridge_transaction = Some(transaction);
    }

    fn merge_runtime(&mut self, delta: Self) {
        self.retired_cpu_slots = self.retired_cpu_slots.wrapping_add(delta.retired_cpu_slots);
        self.elapsed_system_cycles = self
            .elapsed_system_cycles
            .wrapping_add(delta.elapsed_system_cycles);
        self.held_cpu_read_system_cycles = self
            .held_cpu_read_system_cycles
            .wrapping_add(delta.held_cpu_read_system_cycles);
        self.reu_dma_system_cycles = self
            .reu_dma_system_cycles
            .wrapping_add(delta.reu_dma_system_cycles);
        self.reu_dma_vic_stall_cycles = self
            .reu_dma_vic_stall_cycles
            .wrapping_add(delta.reu_dma_vic_stall_cycles);
        self.reu_dma_bus_cycles = self
            .reu_dma_bus_cycles
            .wrapping_add(delta.reu_dma_bus_cycles);
        self.cpu_bus_transactions = self
            .cpu_bus_transactions
            .wrapping_add(delta.cpu_bus_transactions);
        if delta.last_cpu_bus_transaction.is_some() {
            self.last_cpu_bus_transaction = delta.last_cpu_bus_transaction;
        }
        if delta.last_bus_bridge_transaction.is_some() {
            self.last_bus_bridge_transaction = delta.last_bus_bridge_transaction;
        }
        self.execution_mode_changes = self
            .execution_mode_changes
            .wrapping_add(delta.execution_mode_changes);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct C64Core {
    config: CoreConfig,
    execution: ExecutionController,
    clock: VirtualClock,
    cpu: Cpu6510,
    address_space: C64AddressSpace,
    devices: C64Chipset,
    irq_line: CpuIrqLine,
    nmi_line: CpuNmiLine,
    #[wincode(skip)]
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

    /// 原子替换前端提供的键盘矩阵、双控制口与 RESTORE 快照。
    ///
    /// # Errors
    ///
    /// 快照的矩阵列数或操纵杆掩码无效时拒绝更新。
    pub fn set_host_input(
        &mut self,
        pressed_rows_by_column: &[u8],
        shift_lock_pressed: bool,
        joystick_port_1_grounded: u8,
        joystick_port_2_grounded: u8,
        restore_key_pressed: bool,
    ) -> Result<(), CoreError> {
        self.devices.set_host_input(
            pressed_rows_by_column,
            shift_lock_pressed,
            joystick_port_1_grounded,
            joystick_port_2_grounded,
            restore_key_pressed,
        )?;
        Ok(())
    }

    pub fn basic_ready(&self) -> bool {
        const READY_SCREEN_CODES: [u8; 6] = [0x12, 0x05, 0x01, 0x04, 0x19, 0x2e];
        const SCREEN_START: u16 = 0x0400;
        const SCREEN_END_EXCLUSIVE: u16 = 0x07e8;
        const READY_SCREEN_CODE_COUNT: u16 = 6;
        let final_start = SCREEN_END_EXCLUSIVE - READY_SCREEN_CODE_COUNT;
        (SCREEN_START..=final_start).any(|start| {
            (start..)
                .zip(READY_SCREEN_CODES)
                .all(|(address, expected)| self.read_base_ram(address) == expected)
        })
    }

    /// 校验并一次性安装一个 `$0801` BASIC PRG，同时排入 PETSCII `RUN`。
    ///
    /// # Errors
    ///
    /// 文件、地址范围或当前 BASIC 键盘缓冲状态无效时，在写 RAM 前返回错误。
    pub fn install_basic_prg(&mut self, bytes: &[u8]) -> Result<LoadedPrg, CoreError> {
        let image = BasicPrgImage::parse(bytes)?;
        let keyboard_buffer_used =
            BasicPrgImage::validate_basic_environment(|address| self.read_base_ram(address))?;

        for (address, value) in
            (image.load_address..image.end_address).zip(image.payload.iter().copied())
        {
            self.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        for pointer in BASIC_TEXT_START_POINTERS {
            self.write_base_ram_word(pointer, image.load_address);
        }
        for pointer in BASIC_TEXT_END_POINTERS {
            self.write_base_ram_word(pointer, image.end_address);
        }
        let keyboard_buffer_start = BASIC_KEYBOARD_BUFFER_START + u16::from(keyboard_buffer_used);
        for (address, value) in (keyboard_buffer_start..).zip(BASIC_RUN_COMMAND) {
            self.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        self.write_base_ram(
            BASIC_KEYBOARD_BUFFER_COUNT,
            keyboard_buffer_used + BASIC_RUN_COMMAND_LENGTH,
            MemoryWriteSource::HostLoader,
        );
        Ok(LoadedPrg {
            load_address: image.load_address,
            end_address: image.end_address,
            size: image.payload.len(),
        })
    }

    /// Parse and attach one CRT cartridge at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects malformed media, unsupported boards, an occupied slot or an
    /// internal CPU slot. Validation completes before the chipset is changed.
    pub fn insert_crt(
        &mut self,
        bytes: &[u8],
        easy_flash_jumper_installed: bool,
    ) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        let cartridge = Cartridge::from_crt_bytes(bytes, easy_flash_jumper_installed)
            .map_err(C64ChipsetError::from)?;
        self.devices.attach_cartridge(cartridge)?;
        Ok(())
    }

    /// Detach and return the expansion-port cartridge at a cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an empty slot or an internal CPU slot.
    pub fn eject_cartridge(&mut self) -> Result<Cartridge, CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.detach_cartridge().map_err(Into::into)
    }

    pub const fn cartridge_attached(&self) -> bool {
        self.devices.cartridge().is_some()
    }

    pub fn cartridge_kind(&self) -> Option<CartridgeKind> {
        self.devices.cartridge().map(Cartridge::kind)
    }

    /// Borrow the attached `EasyFlash` board at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot, an empty slot or another cartridge type.
    pub fn easyflash_mut(
        &mut self,
    ) -> Result<&mut crate::devices::cartridge::EasyFlashCartridge, CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices
            .cartridge_mut()
            .and_then(Cartridge::easyflash_mut)
            .ok_or(C64ChipsetError::EasyFlashNotAttached.into())
    }

    pub fn easyflash_dirty(&self) -> Option<bool> {
        self.devices
            .cartridge()
            .and_then(Cartridge::easyflash)
            .map(|cartridge| cartridge.flash_low().dirty() || cartridge.flash_high().dirty())
    }

    /// Attach a blank classic 17xx REU at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an occupied expansion port or an internal CPU slot.
    pub fn attach_reu(&mut self, size: ReuSize) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.attach_reu(RamExpansionUnit::new(size))?;
        Ok(())
    }

    /// Attach a classic 17xx REU initialized from a physical DRAM image.
    ///
    /// # Errors
    ///
    /// Rejects an image of the wrong size, an occupied expansion port or an
    /// internal CPU slot.
    pub fn attach_reu_image(&mut self, size: ReuSize, image: &[u8]) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        let reu = RamExpansionUnit::from_image(size, image).map_err(C64ChipsetError::from)?;
        self.devices.attach_reu(reu)?;
        Ok(())
    }

    /// Detach and return the physical REU at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an empty expansion port or an internal CPU slot.
    pub fn detach_reu(&mut self) -> Result<RamExpansionUnit, CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.detach_reu().map_err(Into::into)
    }

    pub const fn reu(&self) -> Option<&RamExpansionUnit> {
        self.devices.reu()
    }

    /// Mutably borrow the attached REU at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an empty expansion port or an internal CPU slot.
    pub fn reu_mut(&mut self) -> Result<&mut RamExpansionUnit, CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices
            .reu_mut()
            .ok_or(C64ChipsetError::ReuNotAttached.into())
    }

    /// Copy the complete physical REU DRAM in one persistence operation.
    ///
    /// # Errors
    ///
    /// Requires an attached REU.
    pub fn export_reu_ram(&self) -> Result<Vec<u8>, CoreError> {
        self.devices
            .reu()
            .map(|reu| reu.ram().to_vec())
            .ok_or(C64ChipsetError::ReuNotAttached.into())
    }

    /// Copy the physical `EasyFlash` ROML chip for persistence.
    ///
    /// # Errors
    ///
    /// Requires an `EasyFlash` cartridge.
    pub fn export_easyflash_low(&self) -> Result<Vec<u8>, CoreError> {
        self.devices
            .cartridge()
            .and_then(Cartridge::easyflash)
            .map(|cartridge| cartridge.flash_low().to_bytes())
            .ok_or(C64ChipsetError::EasyFlashNotAttached.into())
    }

    /// Copy the physical `EasyFlash` ROMH chip for persistence.
    ///
    /// # Errors
    ///
    /// Requires an `EasyFlash` cartridge.
    pub fn export_easyflash_high(&self) -> Result<Vec<u8>, CoreError> {
        self.devices
            .cartridge()
            .and_then(Cartridge::easyflash)
            .map(|cartridge| cartridge.flash_high().to_bytes())
            .ok_or(C64ChipsetError::EasyFlashNotAttached.into())
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

    pub const fn drive1541_attached(&self) -> bool {
        self.devices.drive1541().is_some()
    }

    pub fn drive1541_disk_mounted(&self) -> bool {
        self.devices
            .drive1541()
            .is_some_and(|drive| drive.machine().mechanism().disk_present())
    }

    pub fn drive1541_disk_write_protected(&self) -> bool {
        self.devices
            .drive1541()
            .is_none_or(|drive| drive.machine().mechanism().write_protected())
    }

    /// Parse and mount one D64 at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot, an empty drive, malformed media or an
    /// occupied mechanism.
    pub fn mount_drive1541_d64(
        &mut self,
        bytes: &[u8],
        write_protected: bool,
    ) -> Result<(), CoreError> {
        self.drive1541_mut()?
            .mount_d64(bytes, write_protected)
            .map_err(C64ChipsetError::from)?;
        Ok(())
    }

    /// Parse and mount one G64 at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot, an empty drive, malformed media or an
    /// occupied mechanism.
    pub fn mount_drive1541_g64(
        &mut self,
        bytes: &[u8],
        write_protected: bool,
    ) -> Result<(), CoreError> {
        self.drive1541_mut()?
            .mount_g64(bytes, write_protected)
            .map_err(C64ChipsetError::from)?;
        Ok(())
    }

    /// Eject and return one drive image at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot, an empty drive/mechanism, and D64 media
    /// with uncommitted raw-track writes.
    pub fn eject_drive1541_disk(&mut self) -> Result<Drive1541DiskImage, CoreError> {
        self.drive1541_mut()?
            .eject_disk()
            .map_err(C64ChipsetError::from)
            .map_err(CoreError::from)
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

    /// Borrow the physical Datasette at a system-cycle boundary for coarse
    /// media and transport operations.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot.
    pub fn datasette_mut(
        &mut self,
    ) -> Result<&mut crate::devices::tape::Commodore1530Datasette, CoreError> {
        self.clock.advance_system_cycles(0)?;
        Ok(self.devices.datasette_mut())
    }

    /// Parse and insert one read-only TAP image at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects malformed media, duplicate insertion, a moving transport or an
    /// internal CPU slot.
    pub fn insert_tap(
        &mut self,
        bytes: &[u8],
        legacy_v0_overflow_pulse_cycles: Option<u32>,
    ) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        let image = TapImage::parse(bytes, legacy_v0_overflow_pulse_cycles)?;
        self.devices
            .datasette_mut()
            .insert_tape(DatasetteTape::ReadOnly(image))?;
        Ok(())
    }

    /// Insert an empty writable TAP v1 image at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects duplicate insertion, a moving transport or an internal CPU slot.
    pub fn insert_blank_tap(&mut self, video_standard: TapVideoStandard) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices
            .datasette_mut()
            .insert_tape(DatasetteTape::Writable(WritableTapImage::new(
                video_standard,
            )))?;
        Ok(())
    }

    /// Serialize and eject the mounted TAP image atomically.
    ///
    /// # Errors
    ///
    /// Requires mounted media, a stopped transport and a system-cycle boundary.
    pub fn eject_tap(&mut self) -> Result<Vec<u8>, CoreError> {
        self.clock.advance_system_cycles(0)?;
        let bytes = self
            .devices
            .datasette()
            .mounted_tape()
            .ok_or(DatasetteError::TapeNotInserted)?
            .to_bytes()?;
        self.devices.datasette_mut().eject_tape()?;
        Ok(bytes)
    }

    /// Engage the physical PLAY key at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot.
    pub fn tape_play(&mut self) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.datasette_mut().press_play();
        Ok(())
    }

    /// Engage the physical RECORD key at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Requires stopped writable media and a system-cycle boundary.
    pub fn tape_record(&mut self) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.datasette_mut().press_record()?;
        Ok(())
    }

    /// Stop the physical transport at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot.
    pub fn tape_stop(&mut self) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.datasette_mut().press_stop();
        Ok(())
    }

    /// Rewind mounted media at a system-cycle boundary.
    ///
    /// # Errors
    ///
    /// Requires stopped mounted media and a system-cycle boundary.
    pub fn tape_rewind(&mut self) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.datasette_mut().rewind_to_start()?;
        Ok(())
    }

    /// Seek to a TAP pulse boundary.
    ///
    /// # Errors
    ///
    /// Requires stopped mounted media, an in-range index and a system-cycle
    /// boundary.
    pub fn tape_seek_pulse(&mut self, pulse_index: usize) -> Result<(), CoreError> {
        self.clock.advance_system_cycles(0)?;
        self.devices.datasette_mut().seek_pulse(pulse_index)?;
        Ok(())
    }

    pub const fn tape_mounted(&self) -> bool {
        self.devices.datasette().mounted_tape().is_some()
    }

    pub const fn tape_writable(&self) -> bool {
        match self.devices.datasette().mounted_tape() {
            Some(tape) => tape.writable(),
            None => false,
        }
    }

    pub fn tape_pulse_count(&self) -> usize {
        self.devices
            .datasette()
            .mounted_tape()
            .map_or(0, |tape| tape.pulses().len())
    }

    pub const fn tape_pulse_index(&self) -> usize {
        self.devices.datasette().pulse_index()
    }

    pub const fn tape_transport(&self) -> DatasetteTransport {
        self.devices.datasette().transport()
    }

    pub const fn tape_motor_active(&self) -> bool {
        self.devices.datasette().motor_active()
    }

    pub const fn tape_sense_switch_closed(&self) -> bool {
        self.devices.datasette().sense_switch_closed()
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
        let mut diagnostics_delta = CoreDiagnostics::default();
        let mut pending_sid_cycles = 0;
        let hardware_error = {
            let mut bus = ClockedCpuBus::<true, true>::new(
                ClockedCpuBusWiring {
                    address_space: &mut self.address_space,
                    devices: &mut self.devices,
                    execution: &mut self.execution,
                    profile: self.config.profile,
                    clock: &mut self.clock,
                    irq_line: &mut self.irq_line,
                    nmi_line: &mut self.nmi_line,
                },
                &mut diagnostics_delta,
                &mut pending_sid_cycles,
            );
            self.cpu.reset(&mut bus);
            bus.take_hardware_error()
        };
        self.devices.clock_sid_cycles(pending_sid_cycles);
        self.diagnostics.merge_runtime(diagnostics_delta);
        if let Some(error) = hardware_error {
            return Err(error.into());
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
        let mut diagnostics_delta = CoreDiagnostics::default();
        let mut pending_sid_cycles = 0;
        let result = (|| {
            let mut elapsed_system_cycles = 0;
            while elapsed_system_cycles < cycles {
                elapsed_system_cycles = elapsed_system_cycles.saturating_add(
                    self.run_cpu_slot(&mut diagnostics_delta, &mut pending_sid_cycles)?,
                );
            }
            Ok(())
        })();
        self.devices.clock_sid_cycles(pending_sid_cycles);
        self.diagnostics.merge_runtime(diagnostics_delta);
        result
    }

    /// 推进指定数量的内部 CPU 槽位。
    ///
    /// # Errors
    ///
    /// 生成的 CPU 架构表与严格执行器不一致时返回内部 CPU 错误。
    pub fn run_cpu_slots(&mut self, slots: u64) -> Result<u64, CoreError> {
        let mut diagnostics_delta = CoreDiagnostics::default();
        let result = self.run_cpu_slots_accumulating(slots, &mut diagnostics_delta);
        self.diagnostics.merge_runtime(diagnostics_delta);
        result
    }

    fn run_cpu_slots_accumulating(
        &mut self,
        slots: u64,
        diagnostics: &mut CoreDiagnostics,
    ) -> Result<u64, CoreError> {
        let mut elapsed_cycles = 0;
        let mut pending_sid_cycles = 0;
        let result = (|| {
            for _ in 0..slots {
                elapsed_cycles += self.run_cpu_slot(diagnostics, &mut pending_sid_cycles)?;
            }
            Ok(elapsed_cycles)
        })();
        self.devices.clock_sid_cycles(pending_sid_cycles);
        result
    }

    #[inline]
    fn run_cpu_slot(
        &mut self,
        diagnostics: &mut CoreDiagnostics,
        pending_sid_cycles: &mut u32,
    ) -> Result<u64, CoreError> {
        let strict = self.execution.status().effective_slots == SlotsPerSystemCycle::STRICT;
        let turbo_controls = self.config.profile != MachineProfile::Stock;
        match (strict, turbo_controls) {
            (true, false) => {
                self.run_cpu_slot_inner::<true, false>(diagnostics, pending_sid_cycles)
            }
            (true, true) => self.run_cpu_slot_inner::<true, true>(diagnostics, pending_sid_cycles),
            (false, false) => {
                self.run_cpu_slot_inner::<false, false>(diagnostics, pending_sid_cycles)
            }
            (false, true) => {
                self.run_cpu_slot_inner::<false, true>(diagnostics, pending_sid_cycles)
            }
        }
    }

    #[inline]
    fn run_strict_cpu_slot(
        &mut self,
        diagnostics: &mut CoreDiagnostics,
        pending_sid_cycles: &mut u32,
    ) -> Result<u64, CoreError> {
        self.run_cpu_slot_inner::<true, false>(diagnostics, pending_sid_cycles)
    }

    #[inline]
    fn run_cpu_slot_inner<const STRICT: bool, const TURBO_CONTROLS: bool>(
        &mut self,
        diagnostics: &mut CoreDiagnostics,
        pending_sid_cycles: &mut u32,
    ) -> Result<u64, CoreError> {
        let mut elapsed_cycles =
            self.service_reu_dma::<STRICT, TURBO_CONTROLS>(diagnostics, pending_sid_cycles)?;
        self.service_pending_interrupt();
        self.service_nmi_vector_takeover();
        let (completed_system_cycle, hardware_error) = {
            let mut bus = ClockedCpuBus::<STRICT, TURBO_CONTROLS>::new(
                ClockedCpuBusWiring {
                    address_space: &mut self.address_space,
                    devices: &mut self.devices,
                    execution: &mut self.execution,
                    profile: self.config.profile,
                    clock: &mut self.clock,
                    irq_line: &mut self.irq_line,
                    nmi_line: &mut self.nmi_line,
                },
                diagnostics,
                pending_sid_cycles,
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
        Ok(elapsed_cycles)
    }

    /// 推进到下一次完整 VIC 帧提交，供 Worker 一次交换整帧和同时间段 PCM。
    ///
    /// # Errors
    ///
    /// 核心执行失败，或在调用方给定的系统周期上限内没有提交新帧时返回错误。
    pub fn run_until_next_video_frame(
        &mut self,
        maximum_system_cycles: u64,
    ) -> Result<u64, CoreError> {
        let mut diagnostics_delta = CoreDiagnostics::default();
        let result = self
            .run_until_next_video_frame_accumulating(maximum_system_cycles, &mut diagnostics_delta);
        self.diagnostics.merge_runtime(diagnostics_delta);
        result
    }

    fn run_until_next_video_frame_accumulating(
        &mut self,
        maximum_system_cycles: u64,
        diagnostics: &mut CoreDiagnostics,
    ) -> Result<u64, CoreError> {
        let initial_generation = self.devices.vic().frame_generation();
        let mut elapsed_system_cycles = 0_u64;
        let mut pending_sid_cycles = 0;
        let fixed_stock_strict = self.config.profile == MachineProfile::Stock
            && self.execution.status().effective_slots == SlotsPerSystemCycle::STRICT;
        let result = (|| {
            while self.devices.vic().frame_generation() == initial_generation
                || self.clock.timestamp().slot != 0
            {
                if elapsed_system_cycles >= maximum_system_cycles {
                    return Err(CoreError::VideoFrameTimeout {
                        maximum_system_cycles,
                    });
                }
                let elapsed = if fixed_stock_strict {
                    self.run_strict_cpu_slot(diagnostics, &mut pending_sid_cycles)?
                } else {
                    self.run_cpu_slot(diagnostics, &mut pending_sid_cycles)?
                };
                elapsed_system_cycles = elapsed_system_cycles.saturating_add(elapsed);
            }
            Ok(elapsed_system_cycles)
        })();
        self.devices.clock_sid_cycles(pending_sid_cycles);
        result
    }

    fn service_reu_dma<const STRICT: bool, const TURBO_CONTROLS: bool>(
        &mut self,
        diagnostics: &mut CoreDiagnostics,
        pending_sid_cycles: &mut u32,
    ) -> Result<u64, CoreError> {
        if !self.devices.reu().is_some_and(RamExpansionUnit::dma_active) {
            return Ok(0);
        }
        self.service_active_reu_dma::<STRICT, TURBO_CONTROLS>(diagnostics, pending_sid_cycles)
    }

    #[cold]
    #[inline(never)]
    fn service_active_reu_dma<const STRICT: bool, const TURBO_CONTROLS: bool>(
        &mut self,
        diagnostics: &mut CoreDiagnostics,
        pending_sid_cycles: &mut u32,
    ) -> Result<u64, CoreError> {
        let mut elapsed_cycles = 0;
        while self.devices.reu().is_some_and(RamExpansionUnit::dma_active) {
            let hardware_error = {
                let mut bus = ClockedCpuBus::<STRICT, TURBO_CONTROLS>::new(
                    ClockedCpuBusWiring {
                        address_space: &mut self.address_space,
                        devices: &mut self.devices,
                        execution: &mut self.execution,
                        profile: self.config.profile,
                        clock: &mut self.clock,
                        irq_line: &mut self.irq_line,
                        nmi_line: &mut self.nmi_line,
                    },
                    diagnostics,
                    pending_sid_cycles,
                );
                bus.clock_hardware_cycle(None);
                bus.take_hardware_error()
            };
            if let Some(error) = hardware_error {
                return Err(error.into());
            }

            if self.devices.aec_low() {
                diagnostics.reu_dma_vic_stall_cycles =
                    diagnostics.reu_dma_vic_stall_cycles.wrapping_add(1);
            } else {
                let operation = self
                    .devices
                    .reu()
                    .and_then(RamExpansionUnit::dma_operation)
                    .expect("an active REU must publish one DMA operation");
                let bus_operation = match operation {
                    ReuDmaOperation::ReadC64 { address } => Some((
                        BusAccessKind::Read,
                        address,
                        self.address_space.cpu_data_bus_latch(),
                    )),
                    ReuDmaOperation::WriteC64 { address, value } => {
                        Some((BusAccessKind::Write, address, value))
                    }
                    ReuDmaOperation::Idle => None,
                };
                let c64_value = if let Some((access, address, value)) = bus_operation {
                    self.devices
                        .clock_sid_cycles(core::mem::take(pending_sid_cycles));
                    let (request, descriptor, response) = {
                        let mut bus = ReuDmaBus::new(
                            &mut self.address_space,
                            &mut self.devices,
                            self.clock.timestamp(),
                            self.clock.next_external_event(),
                        );
                        let (request, descriptor) = bus.request(access, address, value);
                        let response = bus.transact(request);
                        (request, descriptor, response)
                    };
                    diagnostics.record_bus_bridge_transaction(BusBridgeTransaction {
                        request,
                        domain: descriptor.domain,
                        response,
                    });
                    diagnostics.reu_dma_bus_cycles = diagnostics.reu_dma_bus_cycles.wrapping_add(1);
                    matches!(access, BusAccessKind::Read).then_some(response.value)
                } else {
                    None
                };
                self.devices
                    .reu_mut()
                    .expect("the active REU cannot detach during DMA")
                    .complete_dma_bus_cycle(c64_value);
            }

            self.clock.advance_external_wait_cycle();
            diagnostics.elapsed_system_cycles = diagnostics.elapsed_system_cycles.wrapping_add(1);
            diagnostics.reu_dma_system_cycles = diagnostics.reu_dma_system_cycles.wrapping_add(1);
            elapsed_cycles += 1;
            let sampled_cycle = self.clock.timestamp().system_cycle;
            self.irq_line
                .update(self.devices.irq_asserted(), sampled_cycle);
            self.nmi_line
                .update(self.devices.nmi_asserted(), sampled_cycle);
        }
        Ok(elapsed_cycles)
    }

    pub fn read_base_ram(&self, address: u16) -> u8 {
        self.address_space.read_base_ram(address)
    }

    pub fn write_base_ram(&mut self, address: u16, value: u8, source: MemoryWriteSource) {
        self.address_space.write_base_ram(address, value, source);
    }

    pub fn pull_sid_samples_into(&mut self, destination: &mut [f32]) -> usize {
        self.devices.sid_mut().pull_samples_into(destination)
    }

    fn write_base_ram_word(&mut self, address: u16, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write_base_ram(address, low, MemoryWriteSource::HostLoader);
        self.write_base_ram(address + 1, high, MemoryWriteSource::HostLoader);
    }

    pub fn save_state(&self) -> Vec<u8> {
        state::encode(self)
    }

    /// 原子载入一个经过完整验证的版本化架构状态。
    ///
    /// # Errors
    ///
    /// 头部、版本、校验和、编码负载或恢复后的硬件状态无效时返回错误。
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), CoreError> {
        match state::decode(bytes)? {
            DecodedState::Full(mut restored) => {
                restored.validate_loaded_state()?;
                restored
                    .address_space
                    .replace_firmware(self.address_space.firmware().clone());
                restored.diagnostics = self.diagnostics;
                restored.diagnostics.state_loads = restored.diagnostics.state_loads.wrapping_add(1);
                *self = *restored;
            }
            DecodedState::Legacy(image) => {
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
            }
        }
        Ok(())
    }

    fn validate_loaded_state(&self) -> Result<(), StateError> {
        if !self.execution.state_is_valid() {
            return Err(StateError::InvalidState(
                "execution request and effective slot count are inconsistent",
            ));
        }
        let effective_slots = self.execution.status().effective_slots;
        if self.clock.slots_per_system_cycle() != effective_slots {
            return Err(StateError::InvalidState(
                "virtual clock and execution controller use different slot counts",
            ));
        }
        let timestamp = self.clock.timestamp();
        if timestamp.slot >= effective_slots.get() {
            return Err(StateError::InvalidClockSlot {
                slot: timestamp.slot,
                slots_per_system_cycle: effective_slots.get(),
            });
        }
        if !self.address_space.state_is_valid() {
            return Err(StateError::InvalidState(
                "address-space storage has an invalid physical size",
            ));
        }
        if !self
            .devices
            .state_matches_config(self.config.video_standard, self.config.sid_model)
        {
            return Err(StateError::InvalidState(
                "chipset state does not match the saved machine configuration",
            ));
        }
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

/// REU-owned C64 bus access kept separate from the CPU's latency-sensitive bus.
///
/// The REU only reaches this bridge while DMA is active. Keeping its request
/// classification and transaction dispatch out of `ClockedCpuBus` lets the
/// normal CPU path remain specialized for `BusMaster::Cpu`.
struct ReuDmaBus<'a> {
    address_space: &'a mut C64AddressSpace,
    devices: &'a mut C64Chipset,
    timestamp: VirtualTimestamp,
    next_external_event: VirtualTimestamp,
}

impl<'a> ReuDmaBus<'a> {
    fn new(
        address_space: &'a mut C64AddressSpace,
        devices: &'a mut C64Chipset,
        timestamp: VirtualTimestamp,
        next_external_event: VirtualTimestamp,
    ) -> Self {
        Self {
            address_space,
            devices,
            timestamp,
            next_external_event,
        }
    }

    fn request(
        &mut self,
        access: BusAccessKind,
        address: u16,
        value: u8,
    ) -> (BusRequest, PageDescriptor) {
        self.address_space
            .synchronize_cpu_mapping(self.devices.cartridge_lines());
        let mut descriptor = if address <= 0x0001 {
            PageDescriptor::FAST_RAM
        } else {
            match access {
                BusAccessKind::Read => self.address_space.memory().classify_read(address),
                BusAccessKind::Write => self.address_space.memory().classify_write(address),
            }
        };
        if descriptor.target == PhysicalTarget::Cartridge && address >> 8 == 0xdf {
            descriptor =
                PageDescriptor::bridged(PhysicalTarget::Reu, matches!(access, BusAccessKind::Read));
        }
        (
            BusRequest {
                timestamp: self.timestamp,
                master: BusMaster::Reu,
                access,
                address: u32::from(address),
                value,
                target: descriptor.target,
            },
            descriptor,
        )
    }
}

impl BusBridge for ReuDmaBus<'_> {
    fn transact(&mut self, request: BusRequest) -> BusResponse {
        debug_assert_eq!(request.master, BusMaster::Reu);
        debug_assert_eq!(request.timestamp, self.timestamp);
        let Ok(address) = u16::try_from(request.address) else {
            return BusResponse {
                value: self.devices.open_bus_value(),
                ..BusResponse::default()
            };
        };
        let mapping_before = self.address_space.memory().mapping_generation();
        let value = match request.access {
            BusAccessKind::Read => self.address_space.reu_dma_bus(self.devices).read(address),
            BusAccessKind::Write => {
                self.address_space
                    .reu_dma_bus(self.devices)
                    .write(address, request.value);
                request.value
            }
        };
        self.address_space
            .synchronize_cpu_mapping(self.devices.cartridge_lines());
        BusResponse {
            value,
            wait_system_cycles: 0,
            mapping_changed: mapping_before != self.address_space.memory().mapping_generation(),
            dma_requested: self.devices.reu().is_some_and(RamExpansionUnit::dma_active),
        }
    }

    fn next_external_event(&self) -> Option<VirtualTimestamp> {
        Some(self.next_external_event)
    }
}

const VM_TURBO_ENABLE_REGISTER: u16 = 0xd030;
const VM_TURBO_SPEED_REGISTER: u16 = 0xd031;
const SUPERCPU_NORMAL_REGISTER: u16 = 0xd07a;
const SUPERCPU_TURBO_REGISTER: u16 = 0xd07b;

const fn turbo_control_is_selected(
    profile: MachineProfile,
    access: BusAccessKind,
    address: u16,
) -> bool {
    match profile {
        MachineProfile::Stock => false,
        MachineProfile::SuperCpu => {
            matches!(access, BusAccessKind::Write)
                && matches!(address, SUPERCPU_NORMAL_REGISTER | SUPERCPU_TURBO_REGISTER)
        }
        MachineProfile::VmEnhanced => match address {
            VM_TURBO_ENABLE_REGISTER | VM_TURBO_SPEED_REGISTER => true,
            SUPERCPU_NORMAL_REGISTER | SUPERCPU_TURBO_REGISTER => {
                matches!(access, BusAccessKind::Write)
            }
            _ => false,
        },
    }
}

struct ProfiledBusDevices<'a> {
    devices: &'a mut C64Chipset,
    profile: MachineProfile,
    execution: ExecutionController,
    pending_turbo_control: &'a mut Option<TurboControlCommand>,
}

impl ProfiledBusDevices<'_> {
    fn turbo_register_read(&self, address: u16) -> Option<u8> {
        if self.profile != MachineProfile::VmEnhanced {
            return None;
        }
        match address {
            VM_TURBO_ENABLE_REGISTER => Some(u8::from(self.execution.status().is_turbo())),
            VM_TURBO_SPEED_REGISTER => {
                Some(vm_enhanced_turbo_index(self.execution.configured_slots()))
            }
            _ => None,
        }
    }

    fn turbo_register_write(&self, address: u16, value: u8) -> Option<TurboControlCommand> {
        match (self.profile, address) {
            (MachineProfile::SuperCpu, SUPERCPU_NORMAL_REGISTER) => {
                Some(TurboControlCommand::SetEnabled {
                    enabled: false,
                    fallback: Some(vm_enhanced_turbo_slots(10)),
                })
            }
            (MachineProfile::SuperCpu, SUPERCPU_TURBO_REGISTER) => {
                Some(TurboControlCommand::SetEnabled {
                    enabled: true,
                    fallback: Some(vm_enhanced_turbo_slots(10)),
                })
            }
            (MachineProfile::VmEnhanced, VM_TURBO_ENABLE_REGISTER) => {
                Some(TurboControlCommand::SetEnabled {
                    enabled: value & 0x01 != 0,
                    fallback: None,
                })
            }
            (MachineProfile::VmEnhanced, VM_TURBO_SPEED_REGISTER) => Some(
                TurboControlCommand::Configure(vm_enhanced_turbo_slots(value)),
            ),
            (MachineProfile::VmEnhanced, SUPERCPU_NORMAL_REGISTER) => {
                Some(TurboControlCommand::SetEnabled {
                    enabled: false,
                    fallback: None,
                })
            }
            (MachineProfile::VmEnhanced, SUPERCPU_TURBO_REGISTER) => {
                Some(TurboControlCommand::SetEnabled {
                    enabled: true,
                    fallback: None,
                })
            }
            _ => None,
        }
    }
}

impl C64BusDevices for ProfiledBusDevices<'_> {
    fn cartridge_lines(&self) -> CartridgeLines {
        C64BusDevices::cartridge_lines(self.devices)
    }

    fn read_io(&mut self, address: u16, open_bus: u8) -> u8 {
        self.turbo_register_read(address)
            .unwrap_or_else(|| C64BusDevices::read_io(self.devices, address, open_bus))
    }

    fn write_io(&mut self, address: u16, value: u8) {
        if let Some(command) = self.turbo_register_write(address, value) {
            *self.pending_turbo_control = Some(command);
        } else {
            C64BusDevices::write_io(self.devices, address, value);
        }
    }

    fn read_cartridge(&mut self, region: CartridgeRegion, address: u16) -> Option<u8> {
        C64BusDevices::read_cartridge(self.devices, region, address)
    }

    fn write_cartridge(&mut self, region: CartridgeRegion, address: u16, value: u8) {
        C64BusDevices::write_cartridge(self.devices, region, address, value);
    }

    fn open_bus_value(&self) -> u8 {
        C64BusDevices::open_bus_value(self.devices)
    }

    fn cpu_read_was_held(&self) -> bool {
        C64BusDevices::cpu_read_was_held(self.devices)
    }

    fn processor_port_input_state(&self) -> crate::processor_port::ProcessorPortInputState {
        C64BusDevices::processor_port_input_state(self.devices)
    }

    fn processor_port_output_changed(
        &mut self,
        state: crate::processor_port::ProcessorPortOutputState,
    ) {
        C64BusDevices::processor_port_output_changed(self.devices, state);
    }

    fn observe_cpu_write(&mut self, address: u16) {
        C64BusDevices::observe_cpu_write(self.devices, address);
    }
}

struct ClockedCpuBusWiring<'a> {
    address_space: &'a mut C64AddressSpace,
    devices: &'a mut C64Chipset,
    execution: &'a mut ExecutionController,
    profile: MachineProfile,
    clock: &'a mut VirtualClock,
    irq_line: &'a mut CpuIrqLine,
    nmi_line: &'a mut CpuNmiLine,
}

struct ClockedCpuBus<'a, const STRICT: bool, const TURBO_CONTROLS: bool> {
    address_space: &'a mut C64AddressSpace,
    devices: &'a mut C64Chipset,
    execution: &'a mut ExecutionController,
    profile: MachineProfile,
    clock: &'a mut VirtualClock,
    irq_line: &'a mut CpuIrqLine,
    nmi_line: &'a mut CpuNmiLine,
    diagnostics: &'a mut CoreDiagnostics,
    pending_sid_cycles: &'a mut u32,
    passive_cpu_read: Option<(u16, u8)>,
    board_inputs_current: bool,
    completed_system_cycle: bool,
    pending_turbo_control: Option<TurboControlCommand>,
    hardware_error: Option<C64ChipsetError>,
}

impl<'a, const STRICT: bool, const TURBO_CONTROLS: bool> ClockedCpuBus<'a, STRICT, TURBO_CONTROLS> {
    fn new(
        wiring: ClockedCpuBusWiring<'a>,
        diagnostics: &'a mut CoreDiagnostics,
        pending_sid_cycles: &'a mut u32,
    ) -> Self {
        let ClockedCpuBusWiring {
            address_space,
            devices,
            execution,
            profile,
            clock,
            irq_line,
            nmi_line,
        } = wiring;
        Self {
            address_space,
            devices,
            execution,
            profile,
            clock,
            irq_line,
            nmi_line,
            diagnostics,
            pending_sid_cycles,
            passive_cpu_read: None,
            board_inputs_current: false,
            completed_system_cycle: false,
            pending_turbo_control: None,
            hardware_error: None,
        }
    }

    const fn completed_system_cycle(&self) -> bool {
        self.completed_system_cycle
    }

    fn take_hardware_error(&mut self) -> Option<C64ChipsetError> {
        self.hardware_error.take()
    }

    fn begin_cpu_slot(&mut self, cpu_read_address: Option<u16>) -> bool {
        self.passive_cpu_read = None;
        if !STRICT && self.clock.timestamp().slot != 0 {
            return false;
        }
        debug_assert!(!STRICT || self.clock.timestamp().slot == 0);
        self.clock_hardware_cycle(cpu_read_address);
        true
    }

    fn clock_hardware_cycle(&mut self, cpu_read_address: Option<u16>) {
        self.address_space.tick_processor_port(1);
        if self.devices.datasette_clock_required() {
            let result = self.devices.clock_datasette();
            if let Err(error) = result
                && self.hardware_error.is_none()
            {
                self.hardware_error = Some(error);
            }
        }
        self.address_space
            .synchronize_processor_port_inputs(self.devices.processor_port_input_state());
        if self.devices.cartridge().is_some() {
            self.devices.clock_cartridge();
        }
        self.devices.clock_cias();
        self.passive_cpu_read = cpu_read_address.and_then(|address| {
            self.address_space
                .drive_passive_cpu_read_data_bus(address, self.devices.cartridge_lines())
                .map(|value| (address, value))
        });
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
        self.defer_sid_cycle();
        if self.devices.drive1541().is_some() {
            let result = self.devices.clock_drive1541();
            if let Err(error) = result
                && self.hardware_error.is_none()
            {
                self.hardware_error = Some(error);
            }
        }
        let sampled_cycle = self.clock.timestamp().system_cycle.saturating_add(1);
        self.irq_line
            .update(self.devices.irq_asserted(), sampled_cycle);
        self.nmi_line
            .update(self.devices.nmi_asserted(), sampled_cycle);
        self.devices.clock_host_input();
    }

    fn defer_sid_cycle(&mut self) {
        if *self.pending_sid_cycles == u32::MAX {
            self.flush_sid_cycles();
        }
        *self.pending_sid_cycles += 1;
    }

    fn flush_sid_cycles(&mut self) {
        self.devices
            .clock_sid_cycles(core::mem::take(self.pending_sid_cycles));
    }

    fn read_cpu_address(&mut self, address: u16, board_inputs_current: bool) -> u8 {
        if TURBO_CONTROLS {
            let mut devices = ProfiledBusDevices {
                devices: self.devices,
                profile: self.profile,
                execution: *self.execution,
                pending_turbo_control: &mut self.pending_turbo_control,
            };
            let mut bus = if board_inputs_current {
                self.address_space.cpu_bus_after_passive_read(&mut devices)
            } else {
                self.address_space.cpu_bus(&mut devices)
            };
            bus.read(address)
        } else {
            let mut bus = if board_inputs_current {
                self.address_space.cpu_bus_after_passive_read(self.devices)
            } else {
                self.address_space.cpu_bus(self.devices)
            };
            bus.read(address)
        }
    }

    fn write_cpu_address(&mut self, address: u16, value: u8) {
        if TURBO_CONTROLS {
            let mut devices = ProfiledBusDevices {
                devices: self.devices,
                profile: self.profile,
                execution: *self.execution,
                pending_turbo_control: &mut self.pending_turbo_control,
            };
            self.address_space
                .cpu_bus(&mut devices)
                .write(address, value);
        } else {
            self.address_space
                .cpu_bus(self.devices)
                .write(address, value);
        }
    }

    fn finish_held_system_cycle(&mut self, cpu_read_address: u16) {
        self.clock.advance_external_wait_cycle();
        self.diagnostics.elapsed_system_cycles =
            self.diagnostics.elapsed_system_cycles.wrapping_add(1);
        self.clock_hardware_cycle(Some(cpu_read_address));
    }

    fn finish_cpu_slot(&mut self) {
        let previous_status = self.execution.status();
        let mut next_execution = *self.execution;
        if let Some(command) = self.pending_turbo_control.take() {
            next_execution.apply_guest_turbo_control(command);
        }
        let next_status = next_execution.status();
        let execution_changed = next_status != previous_status;
        let effective_speed_changed =
            next_status.effective_slots != previous_status.effective_slots;

        if STRICT {
            self.clock.consume_strict_cpu_slot();
            self.completed_system_cycle = true;
        } else if effective_speed_changed {
            self.clock.finish_turbo_system_cycle();
            self.completed_system_cycle = true;
        } else {
            self.completed_system_cycle = self.clock.consume_cpu_slot();
        }
        if execution_changed {
            if effective_speed_changed {
                self.clock
                    .set_slots_per_system_cycle(next_status.effective_slots)
                    .expect("a guest speed change must commit at a system-cycle boundary");
            }
            *self.execution = next_execution;
            self.diagnostics.execution_mode_changes =
                self.diagnostics.execution_mode_changes.wrapping_add(1);
        }
        self.diagnostics.retired_cpu_slots = self.diagnostics.retired_cpu_slots.wrapping_add(1);
        if self.completed_system_cycle {
            self.diagnostics.elapsed_system_cycles =
                self.diagnostics.elapsed_system_cycles.wrapping_add(1);
        }
    }

    fn cpu_bus_request(
        &mut self,
        access: BusAccessKind,
        address: u16,
        value: u8,
    ) -> (BusRequest, PageDescriptor) {
        self.address_space
            .synchronize_cpu_mapping(self.devices.cartridge_lines());
        let mut descriptor = match access {
            BusAccessKind::Read => self.address_space.memory().classify_read(address),
            BusAccessKind::Write => self.address_space.memory().classify_write(address),
        };
        if TURBO_CONTROLS
            && descriptor.target == PhysicalTarget::Vic
            && turbo_control_is_selected(self.profile, access, address)
        {
            descriptor = PageDescriptor::bridged(
                PhysicalTarget::TurboControl,
                matches!(access, BusAccessKind::Read),
            );
        } else if self.devices.reu().is_some()
            && descriptor.target == PhysicalTarget::Cartridge
            && address >> 8 == 0xdf
        {
            descriptor =
                PageDescriptor::bridged(PhysicalTarget::Reu, matches!(access, BusAccessKind::Read));
        } else if self.devices.reu().is_some()
            && matches!(access, BusAccessKind::Write)
            && address == 0xff00
        {
            descriptor = PageDescriptor::bridged(PhysicalTarget::Reu, false);
        }
        (
            BusRequest {
                timestamp: self.clock.timestamp(),
                master: BusMaster::Cpu,
                access,
                address: u32::from(address),
                value,
                target: descriptor.target,
            },
            descriptor,
        )
    }
}

impl<const STRICT: bool, const TURBO_CONTROLS: bool> BusBridge
    for ClockedCpuBus<'_, STRICT, TURBO_CONTROLS>
{
    fn transact(&mut self, request: BusRequest) -> BusResponse {
        debug_assert_eq!(request.master, BusMaster::Cpu);
        debug_assert_eq!(request.timestamp, self.clock.timestamp());
        let Ok(address) = u16::try_from(request.address) else {
            return BusResponse {
                value: self.devices.open_bus_value(),
                ..BusResponse::default()
            };
        };
        let mapping_before = self.address_space.memory().mapping_generation();
        let value = match request.access {
            BusAccessKind::Read => {
                if let Some((passive_address, value)) = self.passive_cpu_read.take()
                    && passive_address == address
                {
                    value
                } else {
                    self.read_cpu_address(address, self.board_inputs_current)
                }
            }
            BusAccessKind::Write => {
                self.write_cpu_address(address, request.value);
                request.value
            }
        };
        BusResponse {
            value,
            wait_system_cycles: 0,
            mapping_changed: mapping_before != self.address_space.memory().mapping_generation(),
            dma_requested: self.devices.reu().is_some_and(RamExpansionUnit::dma_active),
        }
    }

    fn next_external_event(&self) -> Option<VirtualTimestamp> {
        Some(self.clock.next_external_event())
    }
}

impl<const STRICT: bool, const TURBO_CONTROLS: bool> CpuBus
    for ClockedCpuBus<'_, STRICT, TURBO_CONTROLS>
{
    fn read(&mut self, address: u16) -> u8 {
        self.devices.begin_cpu_read();
        let held_cycles_before = self.diagnostics.held_cpu_read_system_cycles;
        let mut board_inputs_current = self.begin_cpu_slot(Some(address));
        while self.devices.ba_low() && self.hardware_error.is_none() {
            self.devices.mark_cpu_read_held();
            self.diagnostics.held_cpu_read_system_cycles =
                self.diagnostics.held_cpu_read_system_cycles.wrapping_add(1);
            self.finish_held_system_cycle(address);
            board_inputs_current = true;
        }
        if address_may_select_sid(address) {
            self.flush_sid_cycles();
        }
        let value = if STRICT {
            if let Some((passive_address, value)) = self.passive_cpu_read.take()
                && passive_address == address
            {
                value
            } else {
                self.read_cpu_address(address, board_inputs_current)
            }
        } else {
            self.board_inputs_current = board_inputs_current;
            let (request, descriptor) = self.cpu_bus_request(
                BusAccessKind::Read,
                address,
                self.address_space.cpu_data_bus_latch(),
            );
            let mut response = self.transact(request);
            response.wait_system_cycles = u32::try_from(
                self.diagnostics
                    .held_cpu_read_system_cycles
                    .wrapping_sub(held_cycles_before),
            )
            .unwrap_or(u32::MAX);
            self.diagnostics
                .record_bus_bridge_transaction(BusBridgeTransaction {
                    request,
                    domain: descriptor.domain,
                    response,
                });
            response.value
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
        if address_may_select_sid(address) {
            self.flush_sid_cycles();
        }
        if STRICT {
            self.write_cpu_address(address, value);
        } else {
            let (request, descriptor) = self.cpu_bus_request(BusAccessKind::Write, address, value);
            let response = self.transact(request);
            self.diagnostics
                .record_bus_bridge_transaction(BusBridgeTransaction {
                    request,
                    domain: descriptor.domain,
                    response,
                });
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

const fn address_may_select_sid(address: u16) -> bool {
    matches!(address >> 8, 0xd4..=0xd7)
}

#[derive(Debug)]
pub enum CoreError {
    Chipset(C64ChipsetError),
    Execution(ExecutionConfigError),
    Clock(VirtualClockError),
    Cpu(Cpu6510Error),
    State(StateError),
    Prg(PrgError),
    VideoFrameTimeout { maximum_system_cycles: u64 },
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

impl From<DatasetteError> for CoreError {
    fn from(error: DatasetteError) -> Self {
        Self::Chipset(error.into())
    }
}

impl From<TapImageError> for CoreError {
    fn from(error: TapImageError) -> Self {
        Self::from(DatasetteError::from(error))
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

impl From<PrgError> for CoreError {
    fn from(error: PrgError) -> Self {
        Self::Prg(error)
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
            Self::Prg(error) => error.fmt(formatter),
            Self::VideoFrameTimeout {
                maximum_system_cycles,
            } => write!(
                formatter,
                "VIC did not commit a frame within {maximum_system_cycles} system cycles"
            ),
            Self::Vic(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use crate::{
        address_space::{BASIC_ROM_BYTES, C64Firmware, CHARACTER_ROM_BYTES, KERNAL_ROM_BYTES},
        architecture::{CoreConfig, ExecutionRequest, MachineProfile, TurboSpeedRequest},
        bus::{BusAccessKind, BusMaster},
        clock::VirtualTimestamp,
        devices::reu::ReuSize,
        memory::{MemoryWriteSource, PageDomain, PhysicalTarget},
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
    fn processor_port_write_reports_an_ordered_bus_bridge_mapping_exit() {
        let mut core = C64Core::default();
        core.request_manual_turbo(2).unwrap();
        // LDA #$07; STA $00. The reset output latch is zero, so enabling the
        // low three outputs changes the PLA banking configuration.
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x07),
            (0x2002, 0x85),
            (0x2003, 0x00),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(5).unwrap();

        let transaction = core
            .diagnostics()
            .last_bus_bridge_transaction
            .expect("STA $00 must publish its final bus transaction");
        assert_eq!(transaction.request.access, BusAccessKind::Write);
        assert_eq!(transaction.request.address, 0x0000);
        assert_eq!(transaction.request.value, 0x07);
        assert_eq!(transaction.domain, PageDomain::BusBridge);
        assert_eq!(transaction.request.target, PhysicalTarget::ProcessorPort);
        assert!(transaction.response.mapping_changed);
        assert!(!transaction.response.dma_requested);
    }

    #[test]
    fn banked_out_reu_io2_is_classified_as_fast_character_rom() {
        let mut core = C64Core::default();
        core.attach_reu(ReuSize::Kib512).unwrap();
        core.request_manual_turbo(2).unwrap();
        // Select LORAM + HIRAM with CHAREN low, then read $df00. The REU is
        // physically attached but IO2 is not selected by the PLA.
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x07),
            (0x2002, 0x85),
            (0x2003, 0x00),
            (0x2004, 0xa9),
            (0x2005, 0x03),
            (0x2006, 0x85),
            (0x2007, 0x01),
            (0x2008, 0xad),
            (0x2009, 0x00),
            (0x200a, 0xdf),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(14).unwrap();

        let transaction = core
            .diagnostics()
            .last_bus_bridge_transaction
            .expect("the final absolute read must publish its classified transaction");
        assert_eq!(transaction.request.master, BusMaster::Cpu);
        assert_eq!(transaction.request.access, BusAccessKind::Read);
        assert_eq!(transaction.request.address, 0xdf00);
        assert_eq!(transaction.request.target, PhysicalTarget::CharacterRom);
        assert_eq!(transaction.domain, PageDomain::Fast);
        assert_eq!(transaction.response.value, 0x00);
    }

    #[test]
    fn reu_command_write_reports_dma_ownership_before_the_next_cpu_slot() {
        let mut core = C64Core::default();
        core.attach_reu(ReuSize::Kib512).unwrap();
        core.request_manual_turbo(2).unwrap();
        // LDA #$90; STA $df01 (execute immediately, C64 -> REU).
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x90),
            (0x2002, 0x8d),
            (0x2003, 0x01),
            (0x2004, 0xdf),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(6).unwrap();

        let transaction = core
            .diagnostics()
            .last_bus_bridge_transaction
            .expect("the REU command must publish its bus transaction");
        assert_eq!(transaction.request.access, BusAccessKind::Write);
        assert_eq!(transaction.request.address, 0xdf01);
        assert_eq!(transaction.request.value, 0x90);
        assert_eq!(transaction.domain, PageDomain::BusBridge);
        assert_eq!(transaction.request.target, PhysicalTarget::Reu);
        assert!(!transaction.response.mapping_changed);
        assert!(transaction.response.dma_requested);
    }

    #[test]
    fn reu_ff00_trigger_cannot_bypass_the_bus_bridge_as_hidden_ram() {
        let mut core = C64Core::default();
        core.attach_reu(ReuSize::Kib512).unwrap();
        core.reu_mut().unwrap().write_register(0xdf01, 0x80);
        core.request_manual_turbo(2).unwrap();
        assert!(core.reu().unwrap().ff00_trigger_armed());
        // LDA #$5a; STA $ff00
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x5a),
            (0x2002, 0x8d),
            (0x2003, 0x00),
            (0x2004, 0xff),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(6).unwrap();

        let transaction = core
            .diagnostics()
            .last_bus_bridge_transaction
            .expect("the $ff00 trigger must publish its bus transaction");
        assert_eq!(transaction.request.access, BusAccessKind::Write);
        assert_eq!(transaction.request.address, 0xff00);
        assert_eq!(transaction.request.value, 0x5a);
        assert_eq!(transaction.domain, PageDomain::BusBridge);
        assert_eq!(transaction.request.target, PhysicalTarget::Reu);
        assert!(transaction.response.dma_requested);
        assert_eq!(core.read_base_ram(0xff00), 0x5a);
    }

    #[test]
    fn reu_dma_master_crosses_the_bridge_and_invalidates_written_code() {
        let mut core = C64Core::default();
        core.attach_reu(ReuSize::Kib512).unwrap();
        core.reu_mut().unwrap().ram_mut()[0] = 0x5a;
        let guard = core.memory().code_page_guard(0x40);
        {
            let reu = core.reu_mut().unwrap();
            reu.write_register(0xdf02, 0x00);
            reu.write_register(0xdf03, 0x40);
            reu.write_register(0xdf04, 0x00);
            reu.write_register(0xdf05, 0x00);
            reu.write_register(0xdf06, 0x00);
            reu.write_register(0xdf07, 0x01);
            reu.write_register(0xdf08, 0x00);
            reu.write_register(0xdf01, 0x91);
        }

        core.run_cpu_slots(1).unwrap();

        assert_eq!(core.read_base_ram(0x4000), 0x5a);
        assert!(!core.memory().code_page_guard_is_current(guard));
        let transaction = core
            .diagnostics()
            .last_bus_bridge_transaction
            .expect("the REU DMA write must publish its ordered bridge transaction");
        assert_eq!(transaction.request.master, BusMaster::Reu);
        assert_eq!(transaction.request.access, BusAccessKind::Write);
        assert_eq!(transaction.request.address, 0x4000);
        assert_eq!(transaction.request.value, 0x5a);
        assert_eq!(transaction.request.target, PhysicalTarget::BaseRam);
        assert_eq!(transaction.domain, PageDomain::Fast);
        assert_eq!(transaction.response.value, 0x5a);
        assert_eq!(transaction.response.wait_system_cycles, 0);
        assert!(!transaction.response.mapping_changed);
        assert!(transaction.response.dma_requested);
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
    fn vm_enhanced_turbo_registers_configure_toggle_and_report_speed() {
        let mut core = C64Core::new(CoreConfig {
            profile: MachineProfile::VmEnhanced,
            ..CoreConfig::default()
        });
        // Configure U64E2 speed index 10 (20 slots), enable it through $d030,
        // disable through the SuperCPU-compatible alias, then re-enable it.
        let program = [
            0xa9, 0x0a, 0x8d, 0x31, 0xd0, 0xa9, 0x01, 0x8d, 0x30, 0xd0, 0xa9, 0xaa, 0x8d, 0x7a,
            0xd0, 0xa9, 0x55, 0x8d, 0x7b, 0xd0, 0xad, 0x30, 0xd0, 0x8d, 0x00, 0x40, 0xad, 0x31,
            0xd0, 0x8d, 0x01, 0x40,
        ];
        for (offset, value) in program.into_iter().enumerate() {
            core.write_base_ram(
                0x2000 + u16::try_from(offset).unwrap(),
                value,
                MemoryWriteSource::HostLoader,
            );
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(6).unwrap();
        let configured = core.execution_status();
        assert_eq!(
            configured.requested,
            ExecutionRequest::Turbo(TurboSpeedRequest::manual(20).unwrap())
        );
        assert!(!configured.is_turbo());

        core.run_cpu_slots(6).unwrap();
        assert_eq!(core.execution_status().effective_slots.get(), 20);
        core.run_cpu_slots(6).unwrap();
        assert!(!core.execution_status().is_turbo());
        assert_eq!(core.timestamp().slot, 0);
        core.run_cpu_slots(6).unwrap();
        assert_eq!(core.execution_status().effective_slots.get(), 20);

        core.run_cpu_slots(4).unwrap();
        assert_eq!(core.cpu_state().accumulator, 1);
        core.run_cpu_slots(8).unwrap();
        assert_eq!(core.cpu_state().accumulator, 0x0a);
        core.run_cpu_slots(4).unwrap();
        assert_eq!(core.read_base_ram(0x4000), 1);
        assert_eq!(core.read_base_ram(0x4001), 0x0a);
    }

    #[test]
    fn coarse_system_cycle_run_reloads_a_guest_changed_turbo_budget() {
        let mut core = C64Core::new(CoreConfig {
            profile: MachineProfile::VmEnhanced,
            ..CoreConfig::default()
        });
        // Enable 20-slot Turbo after twelve Strict cycles, then stay in a
        // three-cycle JMP loop. The coarse call must still advance twenty
        // legacy system cycles rather than merely retire twenty CPU slots.
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x0a),
            (0x2002, 0x8d),
            (0x2003, 0x31),
            (0x2004, 0xd0),
            (0x2005, 0xa9),
            (0x2006, 0x01),
            (0x2007, 0x8d),
            (0x2008, 0x30),
            (0x2009, 0xd0),
            (0x200a, 0x4c),
            (0x200b, 0x0a),
            (0x200c, 0x20),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));
        let started_at = core.timestamp();

        core.run_system_cycles(20).unwrap();

        assert_eq!(
            core.timestamp(),
            VirtualTimestamp {
                system_cycle: started_at.system_cycle + 20,
                slot: 0,
            }
        );
        assert_eq!(core.execution_status().effective_slots.get(), 20);
    }

    #[test]
    fn vm_speed_register_reports_the_locked_auto_tier() {
        let mut core = C64Core::new(CoreConfig {
            profile: MachineProfile::VmEnhanced,
            ..CoreConfig::default()
        });
        core.request_auto_turbo(48).unwrap();
        core.lock_auto_turbo(24).unwrap();
        for (address, value) in [
            (0x2000, 0xad),
            (0x2001, 0x31),
            (0x2002, 0xd0),
            (0x2003, 0x8d),
            (0x2004, 0x00),
            (0x2005, 0x40),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(8).unwrap();

        assert_eq!(core.read_base_ram(0x4000), 0x0b);
    }

    #[test]
    fn vm_speed_aliases_preserve_the_locked_auto_tier_while_disabled() {
        let mut core = C64Core::new(CoreConfig {
            profile: MachineProfile::VmEnhanced,
            ..CoreConfig::default()
        });
        core.request_auto_turbo(48).unwrap();
        core.lock_auto_turbo(24).unwrap();
        // LDA #$00; STA $d07a; STA $d07b.
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x00),
            (0x2002, 0x8d),
            (0x2003, 0x7a),
            (0x2004, 0xd0),
            (0x2005, 0x8d),
            (0x2006, 0x7b),
            (0x2007, 0xd0),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(6).unwrap();
        assert_eq!(
            core.execution_status().requested,
            ExecutionRequest::Turbo(TurboSpeedRequest::manual(24).unwrap())
        );
        assert!(!core.execution_status().is_turbo());

        let saved = core.save_state();
        let mut restored = C64Core::new(CoreConfig {
            profile: MachineProfile::VmEnhanced,
            ..CoreConfig::default()
        });
        restored.load_state(&saved).unwrap();
        core = restored;

        core.run_cpu_slots(4).unwrap();
        assert_eq!(core.execution_status().effective_slots.get(), 24);
    }

    #[test]
    fn supercpu_speed_alias_ends_the_current_turbo_cycle_at_an_integer_boundary() {
        let mut core = C64Core::new(CoreConfig {
            profile: MachineProfile::SuperCpu,
            ..CoreConfig::default()
        });
        // LDA #$00; STA $d07b; STA $d07a. The written value is irrelevant.
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x00),
            (0x2002, 0x8d),
            (0x2003, 0x7b),
            (0x2004, 0xd0),
            (0x2005, 0x8d),
            (0x2006, 0x7a),
            (0x2007, 0xd0),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(6).unwrap();
        assert_eq!(core.execution_status().effective_slots.get(), 20);
        let enabled_at = core.timestamp();
        assert_eq!(enabled_at.slot, 0);

        core.run_cpu_slots(4).unwrap();
        assert!(!core.execution_status().is_turbo());
        assert_eq!(
            core.timestamp(),
            VirtualTimestamp {
                system_cycle: enabled_at.system_cycle + 1,
                slot: 0,
            }
        );
    }

    #[test]
    fn stock_profile_does_not_expose_accelerator_speed_registers() {
        let mut core = C64Core::default();
        // LDA #$0f; STA $d031; STA $d07b.
        for (address, value) in [
            (0x2000, 0xa9),
            (0x2001, 0x0f),
            (0x2002, 0x8d),
            (0x2003, 0x31),
            (0x2004, 0xd0),
            (0x2005, 0x8d),
            (0x2006, 0x7b),
            (0x2007, 0xd0),
        ] {
            core.write_base_ram(address, value, MemoryWriteSource::HostLoader);
        }
        assert!(core.set_cpu_program_counter(0x2000));

        core.run_cpu_slots(10).unwrap();

        assert_eq!(core.execution_status().requested, ExecutionRequest::Strict);
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
    fn coarse_frame_run_matches_single_slot_execution_exactly() {
        let mut coarse = C64Core::default();
        let mut stepped = coarse.clone();

        let coarse_elapsed = coarse.run_until_next_video_frame(20_000).unwrap();
        let initial_generation = stepped.devices().vic().frame_generation();
        let mut stepped_elapsed = 0_u64;
        while stepped.devices().vic().frame_generation() == initial_generation
            || stepped.timestamp().slot != 0
        {
            stepped_elapsed += stepped.run_cpu_slots(1).unwrap();
            assert!(stepped_elapsed <= 20_000);
        }

        assert_eq!(coarse_elapsed, stepped_elapsed);
        assert_eq!(coarse, stepped);
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
