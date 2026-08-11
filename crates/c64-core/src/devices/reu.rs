// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 17xx RAM Expansion Unit
//
//   File:       reu.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

const REGISTER_MASK: u16 = 0x001f;
const FIRST_UNUSED_REGISTER: u16 = 0x000b;

const STATUS_256K_DRAMS: u8 = 0x10;
const STATUS_VERIFY_ERROR: u8 = 0x20;
const STATUS_END_OF_BLOCK: u8 = 0x40;
const STATUS_INTERRUPT_PENDING: u8 = 0x80;

const COMMAND_TRANSFER_TYPE_MASK: u8 = 0x03;
const COMMAND_FF00_TRIGGER_DISABLED: u8 = 0x10;
const COMMAND_AUTOLOAD: u8 = 0x20;
const COMMAND_EXECUTE: u8 = 0x80;

const INTERRUPT_VERIFY: u8 = 0x20;
const INTERRUPT_END_OF_BLOCK: u8 = 0x40;
const INTERRUPT_ENABLED: u8 = 0x80;
const INTERRUPT_UNUSED_BITS: u8 = 0x1f;

const ADDRESS_CONTROL_FIX_REU: u8 = 0x40;
const ADDRESS_CONTROL_FIX_C64: u8 = 0x80;
const ADDRESS_CONTROL_UNUSED_BITS: u8 = 0x3f;

const CLASSIC_REU_ADDRESS_MASK: u32 = 0x0007_ffff;
const CLASSIC_REU_BANK_UNUSED_BITS: u8 = 0xf8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ReuSize {
    Kib128 = 128,
    Kib256 = 256,
    Kib512 = 512,
}

impl ReuSize {
    pub const fn kibibytes(self) -> u16 {
        self as u16
    }

    pub const fn byte_length(self) -> usize {
        match self {
            Self::Kib128 => 128 * 1024,
            Self::Kib256 => 256 * 1024,
            Self::Kib512 => 512 * 1024,
        }
    }

    pub const fn from_kibibytes(value: u16) -> Option<Self> {
        match value {
            128 => Some(Self::Kib128),
            256 => Some(Self::Kib256),
            512 => Some(Self::Kib512),
            _ => None,
        }
    }

    const fn status_preset(self) -> u8 {
        match self {
            Self::Kib128 => 0,
            Self::Kib256 | Self::Kib512 => STATUS_256K_DRAMS,
        }
    }

    const fn rec_wrap_address(self) -> u32 {
        match self {
            Self::Kib128 => 0x0002_0000,
            Self::Kib256 | Self::Kib512 => 0x0008_0000,
        }
    }

    const fn dram_address_mask(self) -> u32 {
        self.rec_wrap_address() - 1
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReuImageError {
    expected: usize,
    actual: usize,
}

impl fmt::Display for ReuImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "REU image must contain exactly {} bytes; received {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for ReuImageError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransferType {
    C64ToReu,
    ReuToC64,
    Swap,
    Verify,
}

impl TransferType {
    const fn from_command(command: u8) -> Self {
        match command & COMMAND_TRANSFER_TYPE_MASK {
            0 => Self::C64ToReu,
            1 => Self::ReuToC64,
            2 => Self::Swap,
            3 => Self::Verify,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DmaPhase {
    Transfer,
    SwapWrite { value_from_reu: u8 },
    VerifyFailureDelay { compare_last_byte: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DmaState {
    transfer_type: TransferType,
    c64_address: u16,
    reu_address: u32,
    remaining: u32,
    c64_step: u16,
    reu_step: u32,
    phase: DmaPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReuDmaOperation {
    ReadC64 { address: u16 },
    WriteC64 { address: u16, value: u8 },
    Idle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RamExpansionUnit {
    size: ReuSize,
    ram: Box<[u8]>,
    status: u8,
    command: u8,
    c64_address: u16,
    reu_address: u16,
    reu_bank: u8,
    transfer_length: u16,
    interrupt_mask: u8,
    address_control: u8,
    c64_address_shadow: u16,
    reu_address_shadow: u16,
    reu_bank_shadow: u8,
    transfer_length_shadow: u16,
    ff00_trigger_armed: bool,
    dma: Option<DmaState>,
    floating_bus_value: u8,
}

impl RamExpansionUnit {
    pub fn new(size: ReuSize) -> Self {
        let mut result = Self {
            size,
            ram: vec![0xff; size.byte_length()].into_boxed_slice(),
            status: 0,
            command: 0,
            c64_address: 0,
            reu_address: 0,
            reu_bank: 0,
            transfer_length: 0,
            interrupt_mask: 0,
            address_control: 0,
            c64_address_shadow: 0,
            reu_address_shadow: 0,
            reu_bank_shadow: 0,
            transfer_length_shadow: 0,
            ff00_trigger_armed: false,
            dma: None,
            floating_bus_value: 0xff,
        };
        result.reset();
        result
    }

    /// Construct a REU whose physical DRAM is initialized from a persistence image.
    ///
    /// # Errors
    ///
    /// The image length must exactly match the selected physical REU size.
    pub fn from_image(size: ReuSize, image: &[u8]) -> Result<Self, ReuImageError> {
        let mut result = Self::new(size);
        result.replace_ram(image)?;
        Ok(result)
    }

    pub const fn size(&self) -> ReuSize {
        self.size
    }

    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }

    /// Replace all physical REU DRAM without changing controller registers.
    ///
    /// # Errors
    ///
    /// The image length must exactly match the attached REU.
    pub fn replace_ram(&mut self, image: &[u8]) -> Result<(), ReuImageError> {
        if image.len() != self.ram.len() {
            return Err(ReuImageError {
                expected: self.ram.len(),
                actual: image.len(),
            });
        }
        self.ram.copy_from_slice(image);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.status = self.size.status_preset();
        self.command = COMMAND_FF00_TRIGGER_DISABLED;
        self.c64_address = 0;
        self.reu_address = 0;
        self.reu_bank = 0;
        self.transfer_length = u16::MAX;
        self.interrupt_mask = INTERRUPT_UNUSED_BITS;
        self.address_control = ADDRESS_CONTROL_UNUSED_BITS;
        self.c64_address_shadow = 0;
        self.reu_address_shadow = 0;
        self.reu_bank_shadow = 0;
        self.transfer_length_shadow = u16::MAX;
        self.ff00_trigger_armed = false;
        self.dma = None;
        self.floating_bus_value = 0xff;
    }

    pub const fn dma_active(&self) -> bool {
        self.dma.is_some()
    }

    pub const fn ff00_trigger_armed(&self) -> bool {
        self.ff00_trigger_armed
    }

    pub const fn irq_line_low(&self) -> bool {
        self.status & STATUS_INTERRUPT_PENDING != 0
    }

    pub fn peek_register(&self, address: u16) -> u8 {
        match address & REGISTER_MASK {
            0x00 => self.status,
            0x01 => self.command,
            0x02 => self.c64_address.to_le_bytes()[0],
            0x03 => self.c64_address.to_le_bytes()[1],
            0x04 => self.reu_address.to_le_bytes()[0],
            0x05 => self.reu_address.to_le_bytes()[1],
            0x06 => self.reu_bank | CLASSIC_REU_BANK_UNUSED_BITS,
            0x07 => self.transfer_length.to_le_bytes()[0],
            0x08 => self.transfer_length.to_le_bytes()[1],
            0x09 => self.interrupt_mask,
            0x0a => self.address_control,
            _ => 0xff,
        }
    }

    pub fn read_register(&mut self, address: u16) -> u8 {
        let offset = address & REGISTER_MASK;
        let result = self.peek_register(offset);
        if offset == 0 {
            self.status &= !(STATUS_VERIFY_ERROR | STATUS_END_OF_BLOCK | STATUS_INTERRUPT_PENDING);
        }
        result
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        if self.dma_active() {
            return;
        }
        match address & REGISTER_MASK {
            0x01 => {
                self.command = value;
                self.ff00_trigger_armed = false;
                if value & COMMAND_EXECUTE != 0 {
                    if value & COMMAND_FF00_TRIGGER_DISABLED != 0 {
                        self.start_dma();
                    } else {
                        self.ff00_trigger_armed = true;
                    }
                }
            }
            0x02 => {
                self.c64_address_shadow = (self.c64_address_shadow & 0xff00) | u16::from(value);
                self.c64_address = self.c64_address_shadow;
            }
            0x03 => {
                self.c64_address_shadow =
                    (self.c64_address_shadow & 0x00ff) | (u16::from(value) << 8);
                self.c64_address = self.c64_address_shadow;
            }
            0x04 => {
                self.reu_address_shadow = (self.reu_address_shadow & 0xff00) | u16::from(value);
                self.reu_address = self.reu_address_shadow;
            }
            0x05 => {
                self.reu_address_shadow =
                    (self.reu_address_shadow & 0x00ff) | (u16::from(value) << 8);
                self.reu_address = self.reu_address_shadow;
            }
            0x06 => {
                self.reu_bank_shadow = value & !CLASSIC_REU_BANK_UNUSED_BITS;
                self.reu_bank = self.reu_bank_shadow;
            }
            0x07 => {
                self.transfer_length_shadow =
                    (self.transfer_length_shadow & 0xff00) | u16::from(value);
                self.transfer_length = self.transfer_length_shadow;
            }
            0x08 => {
                self.transfer_length_shadow =
                    (self.transfer_length_shadow & 0x00ff) | (u16::from(value) << 8);
                self.transfer_length = self.transfer_length_shadow;
            }
            0x09 => {
                self.interrupt_mask = value | INTERRUPT_UNUSED_BITS;
                self.update_pending_interrupt_from_latched_status();
            }
            0x0a => self.address_control = value | ADDRESS_CONTROL_UNUSED_BITS,
            _ => {}
        }
    }

    pub fn observe_cpu_write(&mut self, address: u16) {
        if address == 0xff00 && self.ff00_trigger_armed && !self.dma_active() {
            self.ff00_trigger_armed = false;
            self.start_dma();
        }
    }

    pub(crate) fn read_io2(&mut self, address: u16) -> Option<u8> {
        (!self.dma_active()).then(|| self.read_register(address & REGISTER_MASK))
    }

    pub(crate) fn write_io2(&mut self, address: u16, value: u8) {
        let offset = address & REGISTER_MASK;
        if !self.dma_active() && offset < FIRST_UNUSED_REGISTER {
            self.write_register(offset, value);
        }
    }

    pub(crate) fn dma_operation(&self) -> Option<ReuDmaOperation> {
        let dma = self.dma.as_ref()?;
        Some(match dma.phase {
            DmaPhase::Transfer => match dma.transfer_type {
                TransferType::C64ToReu | TransferType::Verify | TransferType::Swap => {
                    ReuDmaOperation::ReadC64 {
                        address: dma.c64_address,
                    }
                }
                TransferType::ReuToC64 => ReuDmaOperation::WriteC64 {
                    address: dma.c64_address,
                    value: self.read_reu_byte(dma.reu_address),
                },
            },
            DmaPhase::SwapWrite { value_from_reu } => ReuDmaOperation::WriteC64 {
                address: dma.c64_address,
                value: value_from_reu,
            },
            DmaPhase::VerifyFailureDelay {
                compare_last_byte: true,
            } => ReuDmaOperation::ReadC64 {
                address: dma.c64_address,
            },
            DmaPhase::VerifyFailureDelay {
                compare_last_byte: false,
            } => ReuDmaOperation::Idle,
        })
    }

    pub(crate) fn complete_dma_bus_cycle(&mut self, c64_value: Option<u8>) {
        let Some(mut dma) = self.dma.take() else {
            return;
        };
        match dma.phase {
            DmaPhase::Transfer => match dma.transfer_type {
                TransferType::C64ToReu => {
                    let value = c64_value.expect("C64-to-REU DMA requires a bus read");
                    self.write_reu_byte(dma.reu_address, value);
                    self.floating_bus_value = value;
                    self.complete_transferred_byte(&mut dma, STATUS_END_OF_BLOCK);
                }
                TransferType::ReuToC64 => {
                    self.floating_bus_value = self.read_reu_byte(dma.reu_address);
                    self.complete_transferred_byte(&mut dma, STATUS_END_OF_BLOCK);
                    if self.dma.is_none() {
                        self.floating_bus_value = self.read_reu_byte(dma.reu_address);
                    }
                }
                TransferType::Swap => {
                    let value_from_c64 = c64_value.expect("swap DMA requires a C64 bus read");
                    let value_from_reu = self.read_reu_byte(dma.reu_address);
                    self.write_reu_byte(dma.reu_address, value_from_c64);
                    self.floating_bus_value = value_from_c64;
                    dma.phase = DmaPhase::SwapWrite { value_from_reu };
                    self.dma = Some(dma);
                }
                TransferType::Verify => {
                    let value_from_c64 = c64_value.expect("verify DMA requires a C64 bus read");
                    let value_from_reu = self.read_reu_byte(dma.reu_address);
                    self.advance_dma_addresses(&mut dma);
                    dma.remaining -= 1;
                    if value_from_c64 == value_from_reu {
                        if dma.remaining == 0 {
                            self.finish_dma(dma, STATUS_END_OF_BLOCK, 1);
                        } else {
                            self.dma = Some(dma);
                        }
                    } else if dma.remaining == 0 {
                        self.finish_dma(dma, STATUS_VERIFY_ERROR | STATUS_END_OF_BLOCK, 1);
                    } else {
                        dma.phase = DmaPhase::VerifyFailureDelay {
                            compare_last_byte: dma.remaining == 1,
                        };
                        self.dma = Some(dma);
                    }
                }
            },
            DmaPhase::SwapWrite { .. } => {
                self.advance_dma_addresses(&mut dma);
                dma.remaining -= 1;
                if dma.remaining == 0 {
                    self.finish_dma(dma, STATUS_END_OF_BLOCK, 1);
                } else {
                    dma.phase = DmaPhase::Transfer;
                    self.dma = Some(dma);
                }
            }
            DmaPhase::VerifyFailureDelay { compare_last_byte } => {
                let mut status = STATUS_VERIFY_ERROR;
                if compare_last_byte {
                    let value_from_c64 =
                        c64_value.expect("verify tail check requires a C64 bus read");
                    if value_from_c64 == self.read_reu_byte(dma.reu_address) {
                        status |= STATUS_END_OF_BLOCK;
                    }
                }
                let remaining = dma.remaining;
                self.finish_dma(dma, status, remaining);
            }
        }
    }

    fn start_dma(&mut self) {
        let length = if self.transfer_length == 0 {
            65_536
        } else {
            u32::from(self.transfer_length)
        };
        self.dma = Some(DmaState {
            transfer_type: TransferType::from_command(self.command),
            c64_address: self.c64_address,
            reu_address: u32::from(self.reu_address) | (u32::from(self.reu_bank) << 16),
            remaining: length,
            c64_step: u16::from(self.address_control & ADDRESS_CONTROL_FIX_C64 == 0),
            reu_step: u32::from(self.address_control & ADDRESS_CONTROL_FIX_REU == 0),
            phase: DmaPhase::Transfer,
        });
    }

    fn complete_transferred_byte(&mut self, dma: &mut DmaState, completion_status: u8) {
        self.advance_dma_addresses(dma);
        dma.remaining -= 1;
        if dma.remaining == 0 {
            self.finish_dma(*dma, completion_status, 1);
        } else {
            self.dma = Some(*dma);
        }
    }

    fn advance_dma_addresses(&self, dma: &mut DmaState) {
        dma.c64_address = dma.c64_address.wrapping_add(dma.c64_step);
        if dma.reu_step == 0 {
            return;
        }
        let upper = dma.reu_address & !CLASSIC_REU_ADDRESS_MASK;
        let mut next = (dma.reu_address & CLASSIC_REU_ADDRESS_MASK) + 1;
        if next == self.size.rec_wrap_address() {
            next = 0;
        }
        dma.reu_address = upper | next;
    }

    fn finish_dma(&mut self, dma: DmaState, status: u8, remaining: u32) {
        self.status |= status;
        if self.command & COMMAND_AUTOLOAD == 0 {
            if self.address_control & ADDRESS_CONTROL_FIX_C64 == 0 {
                self.c64_address = dma.c64_address;
            }
            if self.address_control & ADDRESS_CONTROL_FIX_REU == 0 {
                let address = dma.reu_address & CLASSIC_REU_ADDRESS_MASK;
                self.reu_address = low_word(address);
                self.reu_bank = ((address >> 16) as u8) & !CLASSIC_REU_BANK_UNUSED_BITS;
            }
            self.transfer_length = low_word(remaining);
        } else {
            self.c64_address = self.c64_address_shadow;
            self.reu_address = self.reu_address_shadow;
            self.reu_bank = self.reu_bank_shadow;
            self.transfer_length = self.transfer_length_shadow;
        }
        self.update_pending_interrupt_from_latched_status();
        self.command = (self.command & !COMMAND_EXECUTE) | COMMAND_FF00_TRIGGER_DISABLED;
        self.ff00_trigger_armed = false;
        self.dma = None;
    }

    fn update_pending_interrupt_from_latched_status(&mut self) {
        if self.interrupt_mask & (INTERRUPT_ENABLED | INTERRUPT_END_OF_BLOCK)
            == INTERRUPT_ENABLED | INTERRUPT_END_OF_BLOCK
            && self.status & STATUS_END_OF_BLOCK != 0
        {
            self.status |= STATUS_INTERRUPT_PENDING;
        }
        if self.interrupt_mask & (INTERRUPT_ENABLED | INTERRUPT_VERIFY)
            == INTERRUPT_ENABLED | INTERRUPT_VERIFY
            && self.status & STATUS_VERIFY_ERROR != 0
        {
            self.status |= STATUS_INTERRUPT_PENDING;
        }
    }

    fn read_reu_byte(&self, address: u32) -> u8 {
        let physical = address & self.size.dram_address_mask();
        usize::try_from(physical)
            .ok()
            .and_then(|index| self.ram.get(index).copied())
            .unwrap_or(self.floating_bus_value)
    }

    fn write_reu_byte(&mut self, address: u32, value: u8) {
        let physical = address & self.size.dram_address_mask();
        if let Ok(index) = usize::try_from(physical)
            && let Some(destination) = self.ram.get_mut(index)
        {
            *destination = value;
        }
    }
}

fn low_word(value: u32) -> u16 {
    let bytes = value.to_le_bytes();
    u16::from_le_bytes([bytes[0], bytes[1]])
}
