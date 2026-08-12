// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 内存取数流水线
//
//   文件:       fetch.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use super::bad_line::{VicMatrixAccess, VicMatrixAccessSource};
use super::bus_schedule::{VicPhi1Fetch, VicPhi2Fetch};
use super::sequencer::VicCycleResult;
use super::timing::{PAL_VIC_TIMING, VicTiming};

pub const VIC_MATRIX_COLUMN_COUNT: usize = 40;
pub const VIC_SPRITE_COUNT: usize = 8;

const EXTENDED_BACKGROUND_ADDRESS_MASK: u16 = 0x39ff;
const EXTENDED_BACKGROUND_IDLE_ADDRESS: u16 = 0x39ff;
const IDLE_ADDRESS: u16 = 0x3fff;
const REFRESH_PAGE_ADDRESS: u16 = 0x3f00;
const ROW_COUNTER_MASK: u8 = 0x07;
const SPRITE_DATA_ADDRESS_SHIFT: u32 = 6;
const SPRITE_POINTER_TABLE_OFFSET: u16 = 0x03f8;
const VIDEO_COUNTER_MASK: u16 = 0x03ff;

const MODE_BITMAP: u8 = 1 << 0;
const MODE_EXTENDED_BACKGROUND: u8 = 1 << 1;

/// VIC-II 看到的 14 位局部地址空间。CIA2 库选择和字符 ROM 窗口由整机总线实现。
pub trait VicMemoryBus {
    fn cpu_data_bus_value(&self) -> u8;
    fn read_vic_byte(&mut self, address_in_bank: u16) -> u8;
    fn read_vic_color(&mut self, index: u16) -> u8;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicFetchRegisters {
    pub bitmap_memory_address: u16,
    pub character_memory_address: u16,
    pub screen_memory_address: u16,
    mode_flags: u8,
}

impl VicFetchRegisters {
    pub const fn new(
        bitmap_memory_address: u16,
        bitmap_mode: bool,
        character_memory_address: u16,
        extended_background_mode: bool,
        screen_memory_address: u16,
    ) -> Self {
        let mut mode_flags = 0;
        if bitmap_mode {
            mode_flags |= MODE_BITMAP;
        }
        if extended_background_mode {
            mode_flags |= MODE_EXTENDED_BACKGROUND;
        }
        Self {
            bitmap_memory_address,
            character_memory_address,
            screen_memory_address,
            mode_flags,
        }
    }

    pub const fn bitmap_mode(self) -> bool {
        self.mode_flags & MODE_BITMAP != 0
    }

    pub const fn extended_background_mode(self) -> bool {
        self.mode_flags & MODE_EXTENDED_BACKGROUND != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VicFetchSnapshot {
    pub color_matrix: [u8; VIC_MATRIX_COLUMN_COUNT],
    pub graphics: [u8; VIC_MATRIX_COLUMN_COUNT],
    pub idle_state: bool,
    pub last_phi1_byte: u8,
    pub last_phi2_byte: u8,
    pub line_sprite_data: [u32; VIC_SPRITE_COUNT],
    pub line_sprite_display_mask: u8,
    pub line_sprite_pointers: [u8; VIC_SPRITE_COUNT],
    pub matrix_index: u8,
    pub refresh_counter: u8,
    pub row_counter: u8,
    pub screen_matrix: [u8; VIC_MATRIX_COLUMN_COUNT],
    pub sprite_data: [u32; VIC_SPRITE_COUNT],
    pub sprite_pointers: [u8; VIC_SPRITE_COUNT],
    pub video_counter: u16,
    pub video_counter_base: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VicFetchError {
    ColumnOutOfRange { column: u8 },
    MatrixColumnMismatch { expected: u8, received: u8 },
    MatrixIndexOutOfRange { index: u8 },
    MatrixScheduleMissing { cycle: u8 },
    SpriteByteIndexOutOfRange { byte_index: u8 },
    SpriteIndexOutOfRange { sprite_index: u8 },
}

impl fmt::Display for VicFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ColumnOutOfRange { column } => write!(
                formatter,
                "VIC-II display column {column} is outside 0-{}",
                VIC_MATRIX_COLUMN_COUNT - 1
            ),
            Self::MatrixColumnMismatch { expected, received } => write!(
                formatter,
                "VIC-II matrix column {received} does not match pipeline index {expected}"
            ),
            Self::MatrixIndexOutOfRange { index } => write!(
                formatter,
                "VIC-II matrix index {index} is outside the 40-column fetch window"
            ),
            Self::MatrixScheduleMissing { cycle } => write!(
                formatter,
                "VIC-II matrix access at cycle {cycle} has no matching phi2 bus schedule"
            ),
            Self::SpriteByteIndexOutOfRange { byte_index } => write!(
                formatter,
                "VIC-II sprite byte index {byte_index} is outside 0-2"
            ),
            Self::SpriteIndexOutOfRange { sprite_index } => write!(
                formatter,
                "VIC-II sprite index {sprite_index} is outside 0-{}",
                VIC_SPRITE_COUNT - 1
            ),
        }
    }
}

impl std::error::Error for VicFetchError {}

/// 消费时序器的半周期计划；所有行缓存都固定容量，严格运行热路径不会分配。
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct VicFetchPipeline {
    color_matrix: [u8; VIC_MATRIX_COLUMN_COUNT],
    graphics: [u8; VIC_MATRIX_COLUMN_COUNT],
    line_sprite_data: [u32; VIC_SPRITE_COUNT],
    line_sprite_pointers: [u8; VIC_SPRITE_COUNT],
    screen_matrix: [u8; VIC_MATRIX_COLUMN_COUNT],
    sprite_data: [u32; VIC_SPRITE_COUNT],
    sprite_pointers: [u8; VIC_SPRITE_COUNT],
    timing: VicTiming,
    line_sprite_display_mask: u8,
    idle_state: bool,
    last_phi1_byte: u8,
    last_phi2_byte: u8,
    matrix_index: u8,
    refresh_counter: u8,
    row_counter: u8,
    video_counter: u16,
    video_counter_base: u16,
}

impl Default for VicFetchPipeline {
    fn default() -> Self {
        Self::new(PAL_VIC_TIMING)
    }
}

impl VicFetchPipeline {
    pub const fn new(timing: VicTiming) -> Self {
        Self {
            color_matrix: [0; VIC_MATRIX_COLUMN_COUNT],
            graphics: [0; VIC_MATRIX_COLUMN_COUNT],
            line_sprite_data: [0; VIC_SPRITE_COUNT],
            line_sprite_pointers: [0; VIC_SPRITE_COUNT],
            screen_matrix: [0; VIC_MATRIX_COLUMN_COUNT],
            sprite_data: [0; VIC_SPRITE_COUNT],
            sprite_pointers: [0; VIC_SPRITE_COUNT],
            timing,
            line_sprite_display_mask: 0,
            idle_state: true,
            last_phi1_byte: 0xff,
            last_phi2_byte: 0xff,
            matrix_index: 0,
            refresh_counter: 0xff,
            row_counter: 0,
            video_counter: 0,
            video_counter_base: 0,
        }
    }

    pub const fn timing(&self) -> VicTiming {
        self.timing
    }

    pub const fn phi1_data_bus_value(&self) -> u8 {
        self.last_phi1_byte
    }

    pub const fn sprite_display_mask(&self) -> u8 {
        self.line_sprite_display_mask
    }

    /// # Errors
    ///
    /// Returns `VicFetchError::ColumnOutOfRange` when `column` is not a display column.
    pub fn screen_matrix_byte(&self, column: u8) -> Result<u8, VicFetchError> {
        Ok(self.screen_matrix[Self::require_column(column)?])
    }

    /// # Errors
    ///
    /// Returns `VicFetchError::ColumnOutOfRange` when `column` is not a display column.
    pub fn color_matrix_nibble(&self, column: u8) -> Result<u8, VicFetchError> {
        Ok(self.color_matrix[Self::require_column(column)?] & 0x0f)
    }

    /// # Errors
    ///
    /// Returns `VicFetchError::ColumnOutOfRange` when `column` is not a display column.
    pub fn graphics_byte(&self, column: u8) -> Result<u8, VicFetchError> {
        Ok(self.graphics[Self::require_column(column)?])
    }

    /// # Errors
    ///
    /// Returns `VicFetchError::SpriteIndexOutOfRange` for an invalid sprite index.
    pub fn sprite_data_word(&self, sprite_index: u8) -> Result<u32, VicFetchError> {
        let index = Self::require_sprite_index(sprite_index)?;
        Ok(self.line_sprite_data[index])
    }

    /// # Errors
    ///
    /// Returns an explicit invariant error if the sequencer supplies an inconsistent matrix or
    /// sprite transaction. Guest-controlled register values never index an array unchecked.
    pub fn execute_cycle<M: VicMemoryBus>(
        &mut self,
        cycle: &VicCycleResult,
        registers: VicFetchRegisters,
        memory: &mut M,
    ) -> Result<(), VicFetchError> {
        if cycle.frame_started() {
            self.begin_frame();
        }
        if cycle.line_started() {
            self.graphics.fill(0);
        }

        self.execute_phi1(cycle, registers, memory)?;
        if cycle.enter_display_state() {
            self.idle_state = false;
        }
        if cycle.cycle == self.timing.fetch.video_counter_reload_cycle {
            self.video_counter = self.video_counter_base;
            self.matrix_index = 0;
            if cycle.reset_row_counter() {
                self.row_counter = 0;
            }
        }
        if let Some(column) = cycle.late_video_counter_reload_column {
            self.video_counter = self.video_counter_base;
            self.matrix_index = column;
        }
        if cycle.cycle == self.timing.fetch.row_counter_update_cycle {
            self.update_row_counter(cycle.bad_line_condition());
        }

        self.execute_phi2(cycle, registers, memory)?;
        if cycle.cycle == self.timing.sprite.line_data_ready_cycle {
            self.line_sprite_data = self.sprite_data;
            self.line_sprite_display_mask = cycle.sprite_display_mask;
            self.line_sprite_pointers = self.sprite_pointers;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.color_matrix.fill(0);
        self.graphics.fill(0);
        self.line_sprite_data.fill(0);
        self.line_sprite_pointers.fill(0);
        self.screen_matrix.fill(0);
        self.sprite_data.fill(0);
        self.sprite_pointers.fill(0);
        self.line_sprite_display_mask = 0;
        self.idle_state = true;
        self.last_phi1_byte = 0xff;
        self.last_phi2_byte = 0xff;
        self.matrix_index = 0;
        self.refresh_counter = 0xff;
        self.row_counter = 0;
        self.video_counter = 0;
        self.video_counter_base = 0;
    }

    pub const fn snapshot(&self) -> VicFetchSnapshot {
        VicFetchSnapshot {
            color_matrix: self.color_matrix,
            graphics: self.graphics,
            idle_state: self.idle_state,
            last_phi1_byte: self.last_phi1_byte,
            last_phi2_byte: self.last_phi2_byte,
            line_sprite_data: self.line_sprite_data,
            line_sprite_display_mask: self.line_sprite_display_mask,
            line_sprite_pointers: self.line_sprite_pointers,
            matrix_index: self.matrix_index,
            refresh_counter: self.refresh_counter,
            row_counter: self.row_counter,
            screen_matrix: self.screen_matrix,
            sprite_data: self.sprite_data,
            sprite_pointers: self.sprite_pointers,
            video_counter: self.video_counter,
            video_counter_base: self.video_counter_base,
        }
    }

    fn begin_frame(&mut self) {
        self.refresh_counter = 0xff;
        self.video_counter = 0;
        self.video_counter_base = 0;
    }

    fn execute_phi1<M: VicMemoryBus>(
        &mut self,
        cycle: &VicCycleResult,
        registers: VicFetchRegisters,
        memory: &mut M,
    ) -> Result<(), VicFetchError> {
        match cycle.bus_schedule.phi1 {
            VicPhi1Fetch::Refresh => {
                self.last_phi1_byte =
                    memory.read_vic_byte(REFRESH_PAGE_ADDRESS + u16::from(self.refresh_counter));
                self.refresh_counter = self.refresh_counter.wrapping_sub(1);
            }
            VicPhi1Fetch::Graphics => self.fetch_graphics(registers, memory)?,
            VicPhi1Fetch::Idle => {
                self.last_phi1_byte = memory.read_vic_byte(IDLE_ADDRESS);
            }
            VicPhi1Fetch::SpritePointer { sprite_index } => {
                let index = Self::require_sprite_index(sprite_index)?;
                self.sprite_pointers[index] = memory.read_vic_byte(
                    registers.screen_memory_address
                        + SPRITE_POINTER_TABLE_OFFSET
                        + u16::from(sprite_index),
                );
                self.last_phi1_byte = self.sprite_pointers[index];
            }
            VicPhi1Fetch::SpriteData {
                sprite_index,
                byte_index,
            } => {
                self.last_phi1_byte = self.fetch_sprite_data(
                    sprite_index,
                    byte_index,
                    cycle.sprite_data_offsets[0],
                    memory,
                    self.last_phi1_byte,
                )?;
            }
        }
        Ok(())
    }

    fn execute_phi2<M: VicMemoryBus>(
        &mut self,
        cycle: &VicCycleResult,
        registers: VicFetchRegisters,
        memory: &mut M,
    ) -> Result<(), VicFetchError> {
        if let Some(matrix_access) = cycle.matrix_access {
            if cycle.bus_schedule.phi2 != Some(VicPhi2Fetch::Matrix) {
                return Err(VicFetchError::MatrixScheduleMissing { cycle: cycle.cycle });
            }
            self.fetch_matrix(matrix_access, registers, memory)?;
            return Ok(());
        }

        let Some(fetch) = cycle.bus_schedule.phi2 else {
            return Ok(());
        };
        let VicPhi2Fetch::SpriteData {
            sprite_index,
            byte_index,
        } = fetch
        else {
            return Ok(());
        };
        self.last_phi2_byte = self.fetch_sprite_data(
            sprite_index,
            byte_index,
            cycle.sprite_data_offsets[1],
            memory,
            self.last_phi2_byte,
        )?;
        Ok(())
    }

    fn fetch_matrix<M: VicMemoryBus>(
        &mut self,
        access: VicMatrixAccess,
        registers: VicFetchRegisters,
        memory: &mut M,
    ) -> Result<(), VicFetchError> {
        let index = self.require_matrix_index()?;
        if index != access.column {
            return Err(VicFetchError::MatrixColumnMismatch {
                expected: index,
                received: access.column,
            });
        }
        let array_index = usize::from(index);
        if access.source == VicMatrixAccessSource::CpuDataBus {
            self.last_phi2_byte = 0xff;
            self.screen_matrix[array_index] = self.last_phi2_byte;
            self.color_matrix[array_index] = memory.cpu_data_bus_value() & 0x0f;
            return Ok(());
        }

        self.last_phi2_byte =
            memory.read_vic_byte(registers.screen_memory_address + self.video_counter);
        self.screen_matrix[array_index] = self.last_phi2_byte;
        self.color_matrix[array_index] = memory.read_vic_color(self.video_counter);
        Ok(())
    }

    fn fetch_graphics<M: VicMemoryBus>(
        &mut self,
        registers: VicFetchRegisters,
        memory: &mut M,
    ) -> Result<(), VicFetchError> {
        if self.idle_state {
            let idle_address = if registers.extended_background_mode() {
                EXTENDED_BACKGROUND_IDLE_ADDRESS
            } else {
                IDLE_ADDRESS
            };
            self.last_phi1_byte = memory.read_vic_byte(idle_address);
            return Ok(());
        }

        let index = self.require_matrix_index()?;
        let array_index = usize::from(index);
        let screen_code = self.screen_matrix[array_index];
        let mut address = if registers.bitmap_mode() {
            registers.bitmap_memory_address
                | (self.video_counter << 3)
                | u16::from(self.row_counter)
        } else {
            registers.character_memory_address
                | (u16::from(screen_code) << 3)
                | u16::from(self.row_counter)
        };
        if registers.extended_background_mode() {
            address &= EXTENDED_BACKGROUND_ADDRESS_MASK;
        }

        self.last_phi1_byte = memory.read_vic_byte(address);
        self.graphics[array_index] = self.last_phi1_byte;
        self.matrix_index += 1;
        self.video_counter = (self.video_counter + 1) & VIDEO_COUNTER_MASK;
        Ok(())
    }

    fn fetch_sprite_data<M: VicMemoryBus>(
        &mut self,
        sprite_index: u8,
        byte_index: u8,
        address_offset: Option<u8>,
        memory: &mut M,
        inactive_bus_value: u8,
    ) -> Result<u8, VicFetchError> {
        let index = Self::require_sprite_index(sprite_index)?;
        if byte_index >= 3 {
            return Err(VicFetchError::SpriteByteIndexOutOfRange { byte_index });
        }
        let value = if let Some(offset) = address_offset {
            memory.read_vic_byte(
                (u16::from(self.sprite_pointers[index]) << SPRITE_DATA_ADDRESS_SHIFT)
                    + u16::from(offset),
            )
        } else if byte_index == 1 {
            memory.read_vic_byte(IDLE_ADDRESS)
        } else {
            inactive_bus_value
        };
        let shift = u32::from(2 - byte_index) * 8;
        let byte_mask = u32::from(u8::MAX) << shift;
        self.sprite_data[index] =
            (self.sprite_data[index] & !byte_mask) | (u32::from(value) << shift);
        Ok(value)
    }

    fn update_row_counter(&mut self, bad_line: bool) {
        if self.row_counter == ROW_COUNTER_MASK {
            self.idle_state = true;
            self.video_counter_base = self.video_counter;
        }
        if !self.idle_state || bad_line {
            self.row_counter = self.row_counter.wrapping_add(1) & ROW_COUNTER_MASK;
            self.idle_state = false;
        }
    }

    fn require_matrix_index(&self) -> Result<u8, VicFetchError> {
        if usize::from(self.matrix_index) >= VIC_MATRIX_COLUMN_COUNT {
            return Err(VicFetchError::MatrixIndexOutOfRange {
                index: self.matrix_index,
            });
        }
        Ok(self.matrix_index)
    }

    fn require_column(column: u8) -> Result<usize, VicFetchError> {
        if usize::from(column) >= VIC_MATRIX_COLUMN_COUNT {
            return Err(VicFetchError::ColumnOutOfRange { column });
        }
        Ok(usize::from(column))
    }

    fn require_sprite_index(sprite_index: u8) -> Result<usize, VicFetchError> {
        if usize::from(sprite_index) >= VIC_SPRITE_COUNT {
            return Err(VicFetchError::SpriteIndexOutOfRange { sprite_index });
        }
        Ok(usize::from(sprite_index))
    }
}

#[cfg(test)]
mod tests {
    use super::{VicFetchPipeline, VicFetchRegisters, VicMemoryBus};
    use crate::devices::vic::{VicCycleSequencer, VicCycleSignals};

    struct PatternMemory {
        cpu_data_bus: u8,
    }

    impl VicMemoryBus for PatternMemory {
        fn cpu_data_bus_value(&self) -> u8 {
            self.cpu_data_bus
        }

        fn read_vic_byte(&mut self, address_in_bank: u16) -> u8 {
            address_in_bank.to_le_bytes()[0]
        }

        fn read_vic_color(&mut self, index: u16) -> u8 {
            index.to_le_bytes()[0] & 0x0f
        }
    }

    #[test]
    fn bad_line_fetches_forty_matrix_and_graphics_bytes_without_allocation() {
        let mut sequencer = VicCycleSequencer::new();
        let mut fetch = VicFetchPipeline::default();
        let mut memory = PatternMemory { cpu_data_bus: 0 };
        let registers = VicFetchRegisters::new(0x2000, false, 0x1000, false, 0x0400);

        for _ in 0..=(0x30_u32 * 63 + 54) {
            let cycle = sequencer.tick(&VicCycleSignals {
                display_enabled: true,
                vertical_scroll: 0,
                ..VicCycleSignals::default()
            });
            fetch.execute_cycle(&cycle, registers, &mut memory).unwrap();
        }

        let snapshot = fetch.snapshot();
        assert_eq!(snapshot.screen_matrix[0], 0x00);
        assert_eq!(snapshot.screen_matrix[39], 0x27);
        assert_eq!(snapshot.graphics[0], 0x00);
        assert_eq!(snapshot.graphics[39], 0x38);
        assert_eq!(snapshot.matrix_index, 40);
        assert_eq!(snapshot.video_counter, 40);
    }
}
