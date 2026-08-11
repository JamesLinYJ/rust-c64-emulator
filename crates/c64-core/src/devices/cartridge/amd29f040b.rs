// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - AMD AM29F040B flash memory
//
//   File:       devices/cartridge/amd29f040b.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const AMD_29F040B_CAPACITY_BYTES: usize = 0x80000;
pub const AMD_29F040B_SECTOR_SIZE_BYTES: usize = 0x10000;
pub const AMD_29F040B_SECTOR_COUNT: usize = 8;
pub const AMD_29F040B_UNLOCK_ADDRESS_1: usize = 0x0555;
pub const AMD_29F040B_UNLOCK_ADDRESS_2: usize = 0x02aa;
pub const AMD_29F040B_STATUS_TOGGLE_BIT: u8 = 1 << 6;
pub const AMD_29F040B_BYTE_PROGRAM_CYCLES: u32 = 7;
pub const AMD_29F040B_SECTOR_ERASE_WINDOW_CYCLES: u32 = 50;
pub const AMD_29F040B_SECTOR_ERASE_CYCLES: u32 = 1_000_000;
pub const AMD_29F040B_CHIP_ERASE_CYCLES: u32 = 8_000_000;

const ADDRESS_MASK: usize = AMD_29F040B_CAPACITY_BYTES - 1;
const UNLOCK_ADDRESS_MASK: usize = 0x07ff;
const MANUFACTURER_ID: u8 = 0x01;
const DEVICE_ID: u8 = 0xa4;
const STATUS_ERASE_TIMER_EXPIRED: u8 = 1 << 3;
const STATUS_TIMEOUT: u8 = 1 << 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum Amd29F040BState {
    Read,
    Unlock1,
    Unlock2,
    Autoselect,
    ByteProgram,
    ByteProgramBusy,
    ByteProgramError,
    EraseUnlock1,
    EraseUnlock2,
    EraseSelect,
    SectorEraseWindow,
    SectorEraseBusy,
    SectorEraseSuspend,
    ChipEraseBusy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
enum BaseState {
    Read,
    Autoselect,
}

impl BaseState {
    const fn command_state(self) -> Amd29F040BState {
        match self {
            Self::Read => Amd29F040BState::Read,
            Self::Autoselect => Amd29F040BState::Autoselect,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
struct PendingByteProgram {
    address: usize,
    value: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Amd29F040BError {
    InvalidImageSize(usize),
    InvalidAddress(usize),
}

impl fmt::Display for Amd29F040BError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidImageSize(actual) => write!(
                formatter,
                "AM29F040B image must contain {AMD_29F040B_CAPACITY_BYTES} bytes; received {actual}"
            ),
            Self::InvalidAddress(address) => write!(
                formatter,
                "AM29F040B address ${address:x} is outside $00000..=${ADDRESS_MASK:05x}"
            ),
        }
    }
}

impl std::error::Error for Amd29F040BError {}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Amd29F040BFlash {
    data: Box<[u8]>,
    base_state: BaseState,
    dirty: bool,
    erase_sector_mask: u8,
    pending_program: Option<PendingByteProgram>,
    remaining_cycles: u32,
    state: Amd29F040BState,
    status_toggle: u8,
}

impl Amd29F040BFlash {
    /// Copy one complete 4-Mbit flash image.
    ///
    /// # Errors
    ///
    /// Rejects images that do not match the physical chip capacity.
    pub fn new(initial_data: &[u8]) -> Result<Self, Amd29F040BError> {
        if initial_data.len() != AMD_29F040B_CAPACITY_BYTES {
            return Err(Amd29F040BError::InvalidImageSize(initial_data.len()));
        }
        Ok(Self {
            data: initial_data.to_vec().into_boxed_slice(),
            base_state: BaseState::Read,
            dirty: false,
            erase_sector_mask: 0,
            pending_program: None,
            remaining_cycles: 0,
            state: Amd29F040BState::Read,
            status_toggle: 0,
        })
    }

    pub const fn state(&self) -> Amd29F040BState {
        self.state
    }

    pub const fn dirty(&self) -> bool {
        self.dirty
    }

    pub const fn is_busy(&self) -> bool {
        matches!(
            self.state,
            Amd29F040BState::ByteProgramBusy
                | Amd29F040BState::ChipEraseBusy
                | Amd29F040BState::SectorEraseBusy
                | Amd29F040BState::SectorEraseWindow
        )
    }

    /// Read data or the command-status bus value at a physical flash address.
    ///
    /// # Errors
    ///
    /// Rejects addresses outside the chip capacity.
    pub fn read(&mut self, address: usize) -> Result<u8, Amd29F040BError> {
        require_address(address)?;
        Ok(self.read_valid(address))
    }

    /// Read the non-volatile array without affecting command-status toggles.
    ///
    /// # Errors
    ///
    /// Rejects addresses outside the chip capacity.
    pub fn peek(&self, address: usize) -> Result<u8, Amd29F040BError> {
        require_address(address)?;
        Ok(self.data[address])
    }

    /// Feed one command or data write into the flash state machine.
    ///
    /// # Errors
    ///
    /// Rejects addresses outside the chip capacity.
    pub fn write(&mut self, address: usize, value: u8) -> Result<(), Amd29F040BError> {
        require_address(address)?;
        self.write_valid(address, value);
        Ok(())
    }

    /// Advance timed program/erase work in host-clock units.
    pub fn clock_cycles(&mut self, cycles: u32) {
        let mut cycles_left = cycles;
        while cycles_left > 0 && self.is_timed_state() {
            let elapsed = cycles_left.min(self.remaining_cycles);
            cycles_left -= elapsed;
            self.remaining_cycles -= elapsed;
            if self.remaining_cycles == 0 {
                self.complete_timed_state();
            }
        }
    }

    /// Abort the current command decoder without changing non-volatile data.
    pub fn reset_command_state(&mut self) {
        self.base_state = BaseState::Read;
        self.erase_sector_mask = 0;
        self.pending_program = None;
        self.remaining_cycles = 0;
        self.state = Amd29F040BState::Read;
        self.status_toggle = 0;
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.data.to_vec()
    }

    pub(super) fn read_valid(&mut self, address: usize) -> u8 {
        match self.state {
            Amd29F040BState::Autoselect => self.read_autoselect(address),
            Amd29F040BState::ByteProgramBusy => self.read_program_status(false),
            Amd29F040BState::ByteProgramError => self.read_program_status(true),
            Amd29F040BState::ChipEraseBusy
            | Amd29F040BState::SectorEraseBusy
            | Amd29F040BState::SectorEraseSuspend
            | Amd29F040BState::SectorEraseWindow => self.read_erase_status(),
            _ => self.data[address],
        }
    }

    pub(super) fn write_valid(&mut self, address: usize, value: u8) {
        match self.state {
            Amd29F040BState::Read => {
                if is_unlock_address_1(address) && value == 0xaa {
                    self.state = Amd29F040BState::Unlock1;
                }
            }
            Amd29F040BState::Unlock1 => {
                self.state = if is_unlock_address_2(address) && value == 0x55 {
                    Amd29F040BState::Unlock2
                } else {
                    self.base_state.command_state()
                };
            }
            Amd29F040BState::Unlock2 => self.handle_unlocked_command(address, value),
            Amd29F040BState::Autoselect | Amd29F040BState::ByteProgramError => {
                if value == 0xf0 {
                    self.enter_read_state();
                } else if is_unlock_address_1(address) && value == 0xaa {
                    self.state = Amd29F040BState::Unlock1;
                }
            }
            Amd29F040BState::ByteProgram => self.begin_byte_program(address, value),
            Amd29F040BState::EraseUnlock1 => {
                self.state = if is_unlock_address_1(address) && value == 0xaa {
                    Amd29F040BState::EraseUnlock2
                } else {
                    self.base_state.command_state()
                };
            }
            Amd29F040BState::EraseUnlock2 => {
                self.state = if is_unlock_address_2(address) && value == 0x55 {
                    Amd29F040BState::EraseSelect
                } else {
                    self.base_state.command_state()
                };
            }
            Amd29F040BState::EraseSelect => self.handle_erase_selection(address, value),
            Amd29F040BState::SectorEraseWindow => {
                if value == 0x30 {
                    self.select_erase_sector(address);
                } else {
                    self.cancel_sector_erase();
                }
            }
            Amd29F040BState::SectorEraseBusy => {
                if value == 0xb0 {
                    self.state = Amd29F040BState::SectorEraseSuspend;
                }
            }
            Amd29F040BState::SectorEraseSuspend => {
                if value == 0x30 {
                    self.state = Amd29F040BState::SectorEraseBusy;
                }
            }
            Amd29F040BState::ByteProgramBusy | Amd29F040BState::ChipEraseBusy => {}
        }
    }

    fn begin_byte_program(&mut self, address: usize, value: u8) {
        let current = self.data[address];
        self.status_toggle = 0;
        self.pending_program = Some(PendingByteProgram { address, value });
        if current & value != value {
            self.state = Amd29F040BState::ByteProgramError;
            return;
        }
        self.remaining_cycles = AMD_29F040B_BYTE_PROGRAM_CYCLES;
        self.state = Amd29F040BState::ByteProgramBusy;
    }

    fn handle_unlocked_command(&mut self, address: usize, value: u8) {
        if !is_unlock_address_1(address) {
            self.state = self.base_state.command_state();
            return;
        }
        match value {
            0x90 => {
                self.base_state = BaseState::Autoselect;
                self.state = Amd29F040BState::Autoselect;
            }
            0xa0 => self.state = Amd29F040BState::ByteProgram,
            0x80 => self.state = Amd29F040BState::EraseUnlock1,
            0xf0 => self.enter_read_state(),
            _ => self.state = self.base_state.command_state(),
        }
    }

    fn handle_erase_selection(&mut self, address: usize, value: u8) {
        self.status_toggle = 0;
        if is_unlock_address_1(address) && value == 0x10 {
            self.remaining_cycles = AMD_29F040B_CHIP_ERASE_CYCLES;
            self.state = Amd29F040BState::ChipEraseBusy;
            return;
        }
        if value == 0x30 {
            self.erase_sector_mask = 0;
            self.select_erase_sector(address);
            self.remaining_cycles = AMD_29F040B_SECTOR_ERASE_WINDOW_CYCLES;
            self.state = Amd29F040BState::SectorEraseWindow;
            return;
        }
        self.state = self.base_state.command_state();
    }

    fn cancel_sector_erase(&mut self) {
        self.erase_sector_mask = 0;
        self.remaining_cycles = 0;
        self.state = self.base_state.command_state();
    }

    fn complete_timed_state(&mut self) {
        match self.state {
            Amd29F040BState::ByteProgramBusy => self.complete_byte_program(),
            Amd29F040BState::SectorEraseWindow => {
                self.remaining_cycles = AMD_29F040B_SECTOR_ERASE_CYCLES;
                self.state = Amd29F040BState::SectorEraseBusy;
            }
            Amd29F040BState::SectorEraseBusy => self.complete_one_sector_erase(),
            Amd29F040BState::ChipEraseBusy => {
                self.data.fill(0xff);
                self.dirty = true;
                self.state = self.base_state.command_state();
            }
            _ => debug_assert!(false, "AM29F040B completed a non-timed state"),
        }
    }

    fn complete_byte_program(&mut self) {
        let Some(pending) = self.pending_program.take() else {
            debug_assert!(false, "AM29F040B byte program has no pending byte");
            self.enter_read_state();
            return;
        };
        if self.data[pending.address] != pending.value {
            self.data[pending.address] = pending.value;
            self.dirty = true;
        }
        self.state = self.base_state.command_state();
    }

    fn complete_one_sector_erase(&mut self) {
        let Some(sector) = first_selected_sector(self.erase_sector_mask) else {
            debug_assert!(false, "AM29F040B sector erase has no selected sector");
            self.state = self.base_state.command_state();
            return;
        };
        let start = sector * AMD_29F040B_SECTOR_SIZE_BYTES;
        self.data[start..start + AMD_29F040B_SECTOR_SIZE_BYTES].fill(0xff);
        self.dirty = true;
        self.erase_sector_mask &= !(1_u8 << sector);
        if self.erase_sector_mask == 0 {
            self.state = self.base_state.command_state();
        } else {
            self.remaining_cycles = AMD_29F040B_SECTOR_ERASE_CYCLES;
        }
    }

    fn enter_read_state(&mut self) {
        self.base_state = BaseState::Read;
        self.pending_program = None;
        self.remaining_cycles = 0;
        self.state = Amd29F040BState::Read;
    }

    fn read_autoselect(&self, address: usize) -> u8 {
        match address & 0xff {
            0 => MANUFACTURER_ID,
            1 => DEVICE_ID,
            2 => 0,
            _ => self.data[address],
        }
    }

    fn read_program_status(&mut self, error: bool) -> u8 {
        let programmed_value = self.pending_program.map_or(0xff, |pending| pending.value);
        let value = ((programmed_value ^ 0x80) & 0x80)
            | self.status_toggle
            | if error { STATUS_TIMEOUT } else { 0 };
        self.status_toggle ^= AMD_29F040B_STATUS_TOGGLE_BIT;
        value
    }

    fn read_erase_status(&mut self) -> u8 {
        let timer = if self.state == Amd29F040BState::SectorEraseWindow {
            0
        } else {
            STATUS_ERASE_TIMER_EXPIRED
        };
        let value = self.status_toggle | timer;
        self.status_toggle ^= AMD_29F040B_STATUS_TOGGLE_BIT;
        value
    }

    fn select_erase_sector(&mut self, address: usize) {
        let sector = address / AMD_29F040B_SECTOR_SIZE_BYTES;
        self.erase_sector_mask |= 1_u8 << sector;
    }

    const fn is_timed_state(&self) -> bool {
        matches!(
            self.state,
            Amd29F040BState::ByteProgramBusy
                | Amd29F040BState::ChipEraseBusy
                | Amd29F040BState::SectorEraseBusy
                | Amd29F040BState::SectorEraseWindow
        )
    }
}

fn require_address(address: usize) -> Result<(), Amd29F040BError> {
    if address >= AMD_29F040B_CAPACITY_BYTES {
        return Err(Amd29F040BError::InvalidAddress(address));
    }
    Ok(())
}

const fn is_unlock_address_1(address: usize) -> bool {
    address & UNLOCK_ADDRESS_MASK == AMD_29F040B_UNLOCK_ADDRESS_1
}

const fn is_unlock_address_2(address: usize) -> bool {
    address & UNLOCK_ADDRESS_MASK == AMD_29F040B_UNLOCK_ADDRESS_2
}

fn first_selected_sector(mask: u8) -> Option<usize> {
    (0..AMD_29F040B_SECTOR_COUNT).find(|sector| mask & (1_u8 << sector) != 0)
}
