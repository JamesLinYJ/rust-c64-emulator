// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - VIC-II sprite DMA counters
//
//   File:       vic/sprite_dma.rs
//
//   Created:    2026-08-10
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

const DATA_ADDRESS_MASK: u8 = 0x3f;
const DATA_END: u8 = 0x3f;
const CRUNCH_AND_MASK: u8 = 0x2a;
const CRUNCH_OR_MASK: u8 = 0x15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct VicSpriteDma {
    pub(super) active: bool,
    pub(super) display_active: bool,
    expansion_flip_flop: bool,
    memory_counter: u8,
    memory_counter_base: u8,
}

impl Default for VicSpriteDma {
    fn default() -> Self {
        Self::new()
    }
}

impl VicSpriteDma {
    pub(super) const fn new() -> Self {
        Self {
            active: false,
            display_active: false,
            expansion_flip_flop: true,
            memory_counter: 0,
            memory_counter_base: 0,
        }
    }

    pub(super) fn start(&mut self) {
        self.active = true;
        self.expansion_flip_flop = true;
        self.memory_counter = 0;
        self.memory_counter_base = 0;
    }

    pub(super) fn update_memory_counter_base(&mut self) {
        if !self.active || !self.expansion_flip_flop {
            return;
        }
        self.memory_counter_base = self.memory_counter;
        if self.memory_counter_base == DATA_END {
            self.active = false;
        }
    }

    pub(super) fn clock_vertical_expansion(&mut self, expanded: bool) {
        if self.active && expanded {
            self.expansion_flip_flop = !self.expansion_flip_flop;
        }
    }

    pub(super) fn clear_vertical_expansion(&mut self, apply_counter_crunch: bool) {
        if self.expansion_flip_flop {
            return;
        }
        if apply_counter_crunch {
            self.memory_counter = (CRUNCH_AND_MASK
                & (self.memory_counter_base & self.memory_counter))
                | (CRUNCH_OR_MASK & (self.memory_counter_base | self.memory_counter));
        }
        self.expansion_flip_flop = true;
    }

    pub(super) fn prepare_display_row(&mut self, start_display: bool) {
        self.memory_counter = self.memory_counter_base;
        if self.active {
            if start_display {
                self.display_active = true;
            }
        } else {
            self.display_active = false;
        }
    }

    pub(super) fn consume_data_byte(&mut self) -> Option<u8> {
        if !self.active {
            return None;
        }
        let offset = self.memory_counter;
        self.memory_counter = self.memory_counter.wrapping_add(1) & DATA_ADDRESS_MASK;
        Some(offset)
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }
}
