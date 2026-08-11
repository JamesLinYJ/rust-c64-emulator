// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - 6510/PLA 连贯地址空间
//
//   文件:       address_space.rs
//
//   创建日期:   2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::cpu::CpuBus;
use crate::devices::vic::VicMemoryBus;
use crate::memory::{
    BASE_PAGE_COUNT, CoherentMemory, MemoryWriteSource, PageDescriptor, PhysicalTarget,
};
use crate::pla::{C64Pla, PlaInputs, PlaTarget, configuration_code};
use crate::processor_port::{ProcessorPort6510, ProcessorPortInputState, ProcessorPortOutputState};

pub const BASIC_ROM_BYTES: usize = 0x2000;
pub const CHARACTER_ROM_BYTES: usize = 0x1000;
pub const KERNAL_ROM_BYTES: usize = 0x2000;
pub const COLOR_RAM_BYTES: usize = 0x0400;
const VIC_BANK_ADDRESS_MASK: u16 = 0xc000;
const VIC_LOCAL_ADDRESS_MASK: u16 = 0x3fff;
const VIC_CHARACTER_ROM_WINDOW_MASK: u16 = 0x7000;
const VIC_CHARACTER_ROM_WINDOW_VALUE: u16 = 0x1000;
const VIC_CHARACTER_ROM_OFFSET_MASK: u16 = 0x0fff;
const VIC_COLOR_RAM_ADDRESS_MASK: u16 = 0x03ff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CartridgeLines {
    pub game_line_high: bool,
    pub exrom_line_high: bool,
}

impl Default for CartridgeLines {
    fn default() -> Self {
        Self::DISCONNECTED
    }
}

impl CartridgeLines {
    pub const DISCONNECTED: Self = Self {
        game_line_high: true,
        exrom_line_high: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CartridgeRegion {
    RomLow,
    RomHigh,
    Io1,
    Io2,
}

pub trait C64BusDevices {
    fn cartridge_lines(&self) -> CartridgeLines {
        CartridgeLines::DISCONNECTED
    }

    fn read_io(&mut self, _address: u16, open_bus: u8) -> u8 {
        open_bus
    }

    fn write_io(&mut self, _address: u16, _value: u8) {}

    fn read_cartridge(&mut self, _region: CartridgeRegion, _address: u16) -> Option<u8> {
        None
    }

    fn write_cartridge(&mut self, _region: CartridgeRegion, _address: u16, _value: u8) {}

    fn open_bus_value(&self) -> u8 {
        0xff
    }

    fn cpu_read_was_held(&self) -> bool {
        false
    }

    fn processor_port_input_state(&self) -> ProcessorPortInputState {
        ProcessorPortInputState::default()
    }

    fn processor_port_output_changed(&mut self, _state: ProcessorPortOutputState) {}
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DisconnectedBusDevices;

impl C64BusDevices for DisconnectedBusDevices {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C64Firmware {
    basic: Box<[u8]>,
    character: Box<[u8]>,
    kernal: Box<[u8]>,
}

impl C64Firmware {
    /// 复制并校验三个 C64 固件镜像。
    ///
    /// # Errors
    ///
    /// 任一镜像长度与公开的 C64 地址窗口不一致时返回错误。
    pub fn new(basic: &[u8], character: &[u8], kernal: &[u8]) -> Result<Self, FirmwareError> {
        Ok(Self {
            basic: copy_firmware("BASIC", basic, BASIC_ROM_BYTES)?,
            character: copy_firmware("character", character, CHARACTER_ROM_BYTES)?,
            kernal: copy_firmware("KERNAL", kernal, KERNAL_ROM_BYTES)?,
        })
    }

    pub fn blank_for_test() -> Self {
        Self {
            basic: zeroed_bytes(BASIC_ROM_BYTES),
            character: zeroed_bytes(CHARACTER_ROM_BYTES),
            kernal: zeroed_bytes(KERNAL_ROM_BYTES),
        }
    }

    pub fn basic(&self) -> &[u8] {
        &self.basic
    }

    pub fn character(&self) -> &[u8] {
        &self.character
    }

    pub fn kernal(&self) -> &[u8] {
        &self.kernal
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirmwareError {
    image: &'static str,
    expected: usize,
    actual: usize,
}

impl fmt::Display for FirmwareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} ROM must contain {} bytes; received {}",
            self.image, self.expected, self.actual
        )
    }
}

impl std::error::Error for FirmwareError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C64AddressSpace {
    memory: CoherentMemory,
    firmware: C64Firmware,
    processor_port: ProcessorPort6510,
    pla: C64Pla,
    color_ram: Box<[u8]>,
    cpu_data_bus_latch: u8,
}

impl C64AddressSpace {
    pub fn new(firmware: C64Firmware) -> Self {
        let processor_port = ProcessorPort6510::new();
        let mut result = Self {
            memory: CoherentMemory::new(),
            firmware,
            processor_port,
            pla: C64Pla::default(),
            color_ram: zeroed_bytes(COLOR_RAM_BYTES),
            cpu_data_bus_latch: 0xff,
        };
        result.memory.write_base_ram(
            0x0000,
            result.processor_port.direction_register(),
            MemoryWriteSource::HostLoader,
        );
        result.memory.write_base_ram(
            0x0001,
            result.processor_port.output_latch(),
            MemoryWriteSource::HostLoader,
        );
        result.refresh_page_descriptors();
        result
    }

    pub const fn memory(&self) -> &CoherentMemory {
        &self.memory
    }

    pub const fn firmware(&self) -> &C64Firmware {
        &self.firmware
    }

    pub fn read_base_ram(&self, address: u16) -> u8 {
        self.memory.read_base_ram(address)
    }

    pub fn write_base_ram(&mut self, address: u16, value: u8, source: MemoryWriteSource) {
        self.memory.write_base_ram(address, value, source);
    }

    pub(crate) fn clone_base_ram(&self) -> Box<[u8; 65_536]> {
        self.memory.clone_base_ram()
    }

    pub(crate) fn restore_base_ram(&mut self, ram: &[u8]) {
        self.memory.restore_base_ram(ram);
    }

    pub const fn processor_port(&self) -> &ProcessorPort6510 {
        &self.processor_port
    }

    pub const fn pla(&self) -> &C64Pla {
        &self.pla
    }

    pub const fn cpu_data_bus_latch(&self) -> u8 {
        self.cpu_data_bus_latch
    }

    pub fn color_ram(&self) -> &[u8] {
        &self.color_ram
    }

    pub fn tick_processor_port(&mut self, cycles: u32) {
        self.processor_port.tick(cycles);
    }

    pub(crate) fn synchronize_processor_port_inputs(&mut self, state: ProcessorPortInputState) {
        self.processor_port.set_input_pins(state.mask, state.value);
    }

    /// 让当前 CPU 读地址在 VIC 的 φ2 C-access 前无副作用地驱动数据总线。
    ///
    /// RAM 与板载 ROM 是组合读目标，可以在 RDY 拉伸期间持续驱动；I/O、卡带与
    /// open-bus 目标可能带有读副作用或外部状态，因此只在真正完成 CPU 读周期时访问。
    pub(crate) fn drive_passive_cpu_read_data_bus(
        &mut self,
        address: u16,
        cartridge_lines: CartridgeLines,
    ) {
        let value = if address == 0x0000 {
            Some(self.processor_port.direction_register())
        } else if address == 0x0001 {
            Some(self.processor_port.data_register())
        } else {
            self.synchronize_pla(cartridge_lines);
            match self.pla.read_target(address) {
                PlaTarget::Ram => Some(self.memory.read_base_ram(address)),
                PlaTarget::BasicRom => Some(self.firmware.basic()[usize::from(address - 0xa000)]),
                PlaTarget::KernalRom => Some(self.firmware.kernal()[usize::from(address - 0xe000)]),
                PlaTarget::CharacterRom => {
                    Some(self.firmware.character()[usize::from(address - 0xd000)])
                }
                PlaTarget::Io
                | PlaTarget::CartridgeLow
                | PlaTarget::CartridgeHigh
                | PlaTarget::OpenBus => None,
            }
        };
        if let Some(value) = value {
            self.cpu_data_bus_latch = value;
        }
    }

    pub fn reset_processor_port(&mut self, lines: CartridgeLines) {
        self.processor_port.reset();
        self.memory.write_base_ram(
            0x0000,
            self.processor_port.direction_register(),
            MemoryWriteSource::Cpu,
        );
        self.memory.write_base_ram(
            0x0001,
            self.processor_port.output_latch(),
            MemoryWriteSource::Cpu,
        );
        self.synchronize_pla(lines);
    }

    pub fn cpu_bus<'a, D: C64BusDevices>(&'a mut self, devices: &'a mut D) -> C64CpuBus<'a, D> {
        C64CpuBus {
            address_space: self,
            devices,
        }
    }

    pub(crate) const fn vic_memory_bus(&self, bank_address: u16) -> C64VicMemoryBus<'_> {
        C64VicMemoryBus {
            address_space: self,
            bank_address: bank_address & VIC_BANK_ADDRESS_MASK,
        }
    }

    fn synchronize_pla(&mut self, lines: CartridgeLines) {
        let inputs = PlaInputs {
            exrom_line_high: lines.exrom_line_high,
            game_line_high: lines.game_line_high,
            processor_port: self.processor_port.banking_configuration(),
        };
        if self.pla.configuration_code() == configuration_code(inputs) {
            return;
        }
        self.pla.configure(inputs);
        self.refresh_page_descriptors();
    }

    fn refresh_page_descriptors(&mut self) {
        let mut descriptors =
            [(PageDescriptor::FAST_RAM, PageDescriptor::FAST_RAM); BASE_PAGE_COUNT];
        for page in 0_u8..=u8::MAX {
            descriptors[usize::from(page)] = (
                descriptor_for_target(page, self.pla.read_target_for_page(page), true),
                descriptor_for_target(page, self.pla.write_target_for_page(page), false),
            );
        }
        self.memory.apply_page_descriptors(&descriptors);
    }
}

pub(crate) struct C64VicMemoryBus<'a> {
    address_space: &'a C64AddressSpace,
    bank_address: u16,
}

impl VicMemoryBus for C64VicMemoryBus<'_> {
    fn cpu_data_bus_value(&self) -> u8 {
        self.address_space.cpu_data_bus_latch
    }

    fn read_vic_byte(&mut self, address_in_bank: u16) -> u8 {
        let physical_address = self.bank_address | (address_in_bank & VIC_LOCAL_ADDRESS_MASK);
        if physical_address & VIC_CHARACTER_ROM_WINDOW_MASK == VIC_CHARACTER_ROM_WINDOW_VALUE {
            return self.address_space.firmware.character
                [usize::from(physical_address & VIC_CHARACTER_ROM_OFFSET_MASK)];
        }
        self.address_space.memory.read_base_ram(physical_address)
    }

    fn read_vic_color(&mut self, index: u16) -> u8 {
        self.address_space.color_ram[usize::from(index & VIC_COLOR_RAM_ADDRESS_MASK)]
    }
}

pub struct C64CpuBus<'a, D: C64BusDevices> {
    address_space: &'a mut C64AddressSpace,
    devices: &'a mut D,
}

impl<D: C64BusDevices> C64CpuBus<'_, D> {
    fn read_pla_target(&mut self, target: PlaTarget, address: u16) -> u8 {
        match target {
            PlaTarget::Ram => self.address_space.memory.read_base_ram(address),
            PlaTarget::BasicRom => {
                self.address_space.firmware.basic()[usize::from(address - 0xa000)]
            }
            PlaTarget::KernalRom => {
                self.address_space.firmware.kernal()[usize::from(address - 0xe000)]
            }
            PlaTarget::CharacterRom => {
                self.address_space.firmware.character()[usize::from(address - 0xd000)]
            }
            PlaTarget::Io => self.read_io(address),
            PlaTarget::CartridgeLow => self
                .devices
                .read_cartridge(CartridgeRegion::RomLow, address)
                .unwrap_or_else(|| self.devices.open_bus_value()),
            PlaTarget::CartridgeHigh => self
                .devices
                .read_cartridge(CartridgeRegion::RomHigh, address)
                .unwrap_or_else(|| self.devices.open_bus_value()),
            PlaTarget::OpenBus => self.devices.open_bus_value(),
        }
    }

    fn write_pla_target(&mut self, target: PlaTarget, address: u16, value: u8) {
        match target {
            PlaTarget::Ram
            | PlaTarget::BasicRom
            | PlaTarget::KernalRom
            | PlaTarget::CharacterRom => {
                self.address_space
                    .memory
                    .write_base_ram(address, value, MemoryWriteSource::Cpu);
            }
            PlaTarget::Io => self.write_io(address, value),
            PlaTarget::CartridgeLow => {
                self.devices
                    .write_cartridge(CartridgeRegion::RomLow, address, value);
            }
            PlaTarget::CartridgeHigh => {
                self.devices
                    .write_cartridge(CartridgeRegion::RomHigh, address, value);
            }
            PlaTarget::OpenBus => {}
        }
    }

    fn read_io(&mut self, address: u16) -> u8 {
        match address >> 8 {
            0xd8..=0xdb => {
                let color = self.address_space.color_ram[usize::from(address - 0xd800)];
                color | (self.devices.open_bus_value() & 0xf0)
            }
            0xde => self
                .devices
                .read_cartridge(CartridgeRegion::Io1, address)
                .unwrap_or_else(|| self.devices.open_bus_value()),
            0xdf => self
                .devices
                .read_cartridge(CartridgeRegion::Io2, address)
                .unwrap_or_else(|| self.devices.open_bus_value()),
            _ => self.devices.read_io(address, self.devices.open_bus_value()),
        }
    }

    fn write_io(&mut self, address: u16, value: u8) {
        match address >> 8 {
            0xd8..=0xdb => {
                self.address_space.color_ram[usize::from(address - 0xd800)] = value & 0x0f;
            }
            0xde => self
                .devices
                .write_cartridge(CartridgeRegion::Io1, address, value),
            0xdf => self
                .devices
                .write_cartridge(CartridgeRegion::Io2, address, value),
            _ => self.devices.write_io(address, value),
        }
    }
}

impl<D: C64BusDevices> CpuBus for C64CpuBus<'_, D> {
    fn read(&mut self, address: u16) -> u8 {
        self.address_space
            .synchronize_processor_port_inputs(self.devices.processor_port_input_state());
        let value = if address == 0x0000 {
            self.address_space.processor_port.direction_register()
        } else if address == 0x0001 {
            self.address_space.processor_port.data_register()
        } else {
            self.address_space
                .synchronize_pla(self.devices.cartridge_lines());
            self.read_pla_target(self.address_space.pla.read_target(address), address)
        };
        self.address_space.cpu_data_bus_latch = value;
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        self.address_space.cpu_data_bus_latch = value;
        if address == 0x0000 || address == 0x0001 {
            let output_changed = if address == 0x0000 {
                self.address_space.processor_port.write_direction(value)
            } else {
                self.address_space.processor_port.write_data(value)
            };
            self.address_space
                .memory
                .write_base_ram(address, value, MemoryWriteSource::Cpu);
            self.address_space
                .synchronize_pla(self.devices.cartridge_lines());
            if output_changed {
                self.devices.processor_port_output_changed(
                    self.address_space.processor_port.output_state(),
                );
            }
            return;
        }

        self.address_space
            .synchronize_pla(self.devices.cartridge_lines());
        self.write_pla_target(self.address_space.pla.write_target(address), address, value);
    }

    fn read_was_held(&self) -> bool {
        self.devices.cpu_read_was_held()
    }
}

fn descriptor_for_target(page: u8, target: PlaTarget, read: bool) -> PageDescriptor {
    match target {
        PlaTarget::Ram => PageDescriptor::FAST_RAM,
        PlaTarget::BasicRom => PageDescriptor::mapped_fast(PhysicalTarget::BasicRom),
        PlaTarget::KernalRom => PageDescriptor::mapped_fast(PhysicalTarget::KernalRom),
        PlaTarget::CharacterRom => PageDescriptor::mapped_fast(PhysicalTarget::CharacterRom),
        PlaTarget::Io => PageDescriptor::bridged(io_target(page), read),
        PlaTarget::CartridgeLow | PlaTarget::CartridgeHigh => {
            PageDescriptor::bridged(PhysicalTarget::Cartridge, read)
        }
        PlaTarget::OpenBus => PageDescriptor::bridged(PhysicalTarget::OpenBus, false),
    }
}

const fn io_target(page: u8) -> PhysicalTarget {
    match page {
        0xd0..=0xd3 => PhysicalTarget::Vic,
        0xd4..=0xd7 => PhysicalTarget::Sid,
        0xd8..=0xdb => PhysicalTarget::ColorRam,
        0xdc => PhysicalTarget::Cia1,
        0xdd => PhysicalTarget::Cia2,
        _ => PhysicalTarget::Cartridge,
    }
}

fn copy_firmware(
    image_name: &'static str,
    bytes: &[u8],
    expected: usize,
) -> Result<Box<[u8]>, FirmwareError> {
    if bytes.len() != expected {
        return Err(FirmwareError {
            image: image_name,
            expected,
            actual: bytes.len(),
        });
    }
    Ok(bytes.to_vec().into_boxed_slice())
}

fn zeroed_bytes(size: usize) -> Box<[u8]> {
    vec![0; size].into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use crate::cpu::{Cpu6510, Cpu6510State, CpuBus};
    use crate::devices::vic::VicMemoryBus;
    use crate::memory::{PageDomain, PhysicalTarget};

    use super::{
        C64AddressSpace, C64BusDevices, C64Firmware, CartridgeLines, DisconnectedBusDevices,
    };

    #[test]
    fn rom_reads_and_hidden_ram_writes_use_distinct_page_capabilities() {
        let mut basic = [0_u8; super::BASIC_ROM_BYTES];
        basic[0] = 0x42;
        let firmware = C64Firmware::new(
            &basic,
            &[0; super::CHARACTER_ROM_BYTES],
            &[0; super::KERNAL_ROM_BYTES],
        )
        .unwrap();
        let mut address_space = C64AddressSpace::new(firmware);
        let mut devices = DisconnectedBusDevices;

        {
            let mut bus = address_space.cpu_bus(&mut devices);
            assert_eq!(bus.read(0xa000), 0x42);
            bus.write(0xa000, 0x99);
            assert_eq!(bus.read(0xa000), 0x42);
            bus.write(0x0000, 0xff);
            bus.write(0x0001, 0x00);
            assert_eq!(bus.read(0xa000), 0x99);
        }

        assert_eq!(
            address_space.memory().classify_read(0xa000).target,
            PhysicalTarget::BaseRam
        );
        assert_eq!(
            address_space.memory().classify_write(0xa000).domain,
            PageDomain::Fast
        );
    }

    #[test]
    fn strict_cpu_executes_through_the_same_pla_address_space() {
        let mut address_space = C64AddressSpace::new(C64Firmware::blank_for_test());
        let mut devices = DisconnectedBusDevices;
        {
            let mut bus = address_space.cpu_bus(&mut devices);
            // LDA #$5a; STA $2000
            for (address, value) in [
                (0x1000, 0xa9),
                (0x1001, 0x5a),
                (0x1002, 0x8d),
                (0x1003, 0x00),
                (0x1004, 0x20),
            ] {
                bus.write(address, value);
            }
        }
        let mut cpu = Cpu6510::new();
        cpu.restore_state(Cpu6510State {
            program_counter: 0x1000,
            ..Cpu6510State::deterministic_power_on()
        });
        let mut instruction_count = 0;
        while instruction_count < 2 {
            let completed = {
                let mut bus = address_space.cpu_bus(&mut devices);
                cpu.clock_cycle(&mut bus).unwrap()
            };
            if completed {
                instruction_count += 1;
            }
        }
        assert_eq!(address_space.memory().read_base_ram(0x2000), 0x5a);
    }

    #[test]
    fn vic_banks_character_rom_color_ram_and_cpu_latch_share_the_board_memory() {
        let character = [0xcc_u8; super::CHARACTER_ROM_BYTES];
        let firmware = C64Firmware::new(
            &[0; super::BASIC_ROM_BYTES],
            &character,
            &[0; super::KERNAL_ROM_BYTES],
        )
        .unwrap();
        let mut address_space = C64AddressSpace::new(firmware);
        address_space.write_base_ram(0x1000, 0x10, crate::memory::MemoryWriteSource::HostLoader);
        address_space.write_base_ram(0x5000, 0x50, crate::memory::MemoryWriteSource::HostLoader);
        address_space.write_base_ram(0x9000, 0x90, crate::memory::MemoryWriteSource::HostLoader);
        address_space.write_base_ram(0xd000, 0xd0, crate::memory::MemoryWriteSource::HostLoader);
        let mut devices = DisconnectedBusDevices;
        {
            let mut cpu_bus = address_space.cpu_bus(&mut devices);
            cpu_bus.write(0xd812, 0xab);
        }

        let mut bank_0 = address_space.vic_memory_bus(0x0000);
        assert_eq!(bank_0.read_vic_byte(0x1000), 0xcc);
        assert_eq!(bank_0.read_vic_color(0x0012), 0x0b);
        assert_eq!(bank_0.cpu_data_bus_value(), 0xab);

        let mut bank_1 = address_space.vic_memory_bus(0x4000);
        assert_eq!(bank_1.read_vic_byte(0x1000), 0x50);
        let mut bank_2 = address_space.vic_memory_bus(0x8000);
        assert_eq!(bank_2.read_vic_byte(0x1000), 0xcc);
        let mut bank_3 = address_space.vic_memory_bus(0xc000);
        assert_eq!(bank_3.read_vic_byte(0x1000), 0xd0);
    }

    #[test]
    fn passive_cpu_reads_drive_ram_and_rom_without_triggering_io() {
        let mut basic = [0_u8; super::BASIC_ROM_BYTES];
        basic[0] = 0x42;
        let firmware = C64Firmware::new(
            &basic,
            &[0; super::CHARACTER_ROM_BYTES],
            &[0; super::KERNAL_ROM_BYTES],
        )
        .unwrap();
        let mut address_space = C64AddressSpace::new(firmware);
        let mut devices = DisconnectedBusDevices;
        {
            let mut bus = address_space.cpu_bus(&mut devices);
            bus.write(0xd019, 0xa5);
        }

        address_space.drive_passive_cpu_read_data_bus(0xd019, CartridgeLines::DISCONNECTED);
        assert_eq!(address_space.cpu_data_bus_latch(), 0xa5);
        address_space.drive_passive_cpu_read_data_bus(0xa000, CartridgeLines::DISCONNECTED);
        assert_eq!(address_space.cpu_data_bus_latch(), 0x42);
        address_space.drive_passive_cpu_read_data_bus(0x0001, CartridgeLines::DISCONNECTED);
        assert_eq!(
            address_space.cpu_data_bus_latch(),
            address_space.processor_port().data_register()
        );
    }

    #[derive(Default)]
    struct UltimaxDevices;

    impl C64BusDevices for UltimaxDevices {
        fn cartridge_lines(&self) -> CartridgeLines {
            CartridgeLines {
                game_line_high: false,
                exrom_line_high: true,
            }
        }

        fn open_bus_value(&self) -> u8 {
            0xa5
        }
    }

    #[test]
    fn ultimax_holes_are_open_bus_and_do_not_modify_ram() {
        let mut address_space = C64AddressSpace::new(C64Firmware::blank_for_test());
        let mut devices = UltimaxDevices;
        {
            let mut bus = address_space.cpu_bus(&mut devices);
            assert_eq!(bus.read(0xa000), 0xa5);
            bus.write(0xa000, 0x55);
        }
        assert_eq!(address_space.memory().read_base_ram(0xa000), 0x00);
    }
}
