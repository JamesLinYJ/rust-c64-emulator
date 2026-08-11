// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 整数虚拟时钟与高速槽位
//
//   文件:       clock.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::architecture::SlotsPerSystemCycle;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct VirtualTimestamp {
    pub system_cycle: u64,
    pub slot: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualClock {
    timestamp: VirtualTimestamp,
    slots_per_system_cycle: SlotsPerSystemCycle,
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualClock {
    pub const fn new() -> Self {
        Self {
            timestamp: VirtualTimestamp {
                system_cycle: 0,
                slot: 0,
            },
            slots_per_system_cycle: SlotsPerSystemCycle::STRICT,
        }
    }

    pub const fn timestamp(self) -> VirtualTimestamp {
        self.timestamp
    }

    pub const fn slots_per_system_cycle(self) -> SlotsPerSystemCycle {
        self.slots_per_system_cycle
    }

    /// 只在系统周期边界切换内部槽位预算。
    ///
    /// # Errors
    ///
    /// 当前位于周期内部槽位时返回 `ModeChangeAwayFromBoundary`。
    pub fn set_slots_per_system_cycle(
        &mut self,
        slots: SlotsPerSystemCycle,
    ) -> Result<(), VirtualClockError> {
        if self.timestamp.slot != 0 {
            return Err(VirtualClockError::ModeChangeAwayFromBoundary {
                slot: self.timestamp.slot,
            });
        }
        self.slots_per_system_cycle = slots;
        Ok(())
    }

    pub fn consume_cpu_slot(&mut self) -> bool {
        self.timestamp.slot += 1;
        if self.timestamp.slot < self.slots_per_system_cycle.get() {
            return false;
        }
        self.timestamp.slot = 0;
        self.timestamp.system_cycle = self.timestamp.system_cycle.wrapping_add(1);
        true
    }

    /// Advance one external C64 system cycle while RDY keeps the current CPU
    /// micro-cycle pending. The internal Turbo slot is intentionally preserved.
    pub fn advance_external_wait_cycle(&mut self) {
        self.timestamp.system_cycle = self.timestamp.system_cycle.wrapping_add(1);
    }

    /// 从边界直接推进完整 legacy 系统周期。
    ///
    /// # Errors
    ///
    /// 当前位于周期内部槽位时返回 `SystemAdvanceAwayFromBoundary`。
    pub fn advance_system_cycles(&mut self, cycles: u64) -> Result<(), VirtualClockError> {
        if self.timestamp.slot != 0 {
            return Err(VirtualClockError::SystemAdvanceAwayFromBoundary {
                slot: self.timestamp.slot,
            });
        }
        self.timestamp.system_cycle = self.timestamp.system_cycle.wrapping_add(cycles);
        Ok(())
    }

    pub fn restore(&mut self, timestamp: VirtualTimestamp, slots: SlotsPerSystemCycle) {
        self.timestamp = timestamp;
        self.slots_per_system_cycle = slots;
    }

    pub fn reset(&mut self) {
        self.timestamp = VirtualTimestamp::default();
        self.slots_per_system_cycle = SlotsPerSystemCycle::STRICT;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualClockError {
    ModeChangeAwayFromBoundary { slot: u8 },
    SystemAdvanceAwayFromBoundary { slot: u8 },
}

impl fmt::Display for VirtualClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ModeChangeAwayFromBoundary { slot } => {
                write!(
                    formatter,
                    "只能在指令/系统周期边界切换速度，当前槽位为 {slot}"
                )
            }
            Self::SystemAdvanceAwayFromBoundary { slot } => {
                write!(formatter, "不能从子周期槽位 {slot} 直接推进完整系统周期")
            }
        }
    }
}

impl std::error::Error for VirtualClockError {}

#[cfg(test)]
mod tests {
    use crate::architecture::SlotsPerSystemCycle;

    use super::{VirtualClock, VirtualTimestamp};

    #[test]
    fn turbo_slots_advance_one_legacy_cycle_only_after_the_declared_budget() {
        let mut clock = VirtualClock::new();
        clock
            .set_slots_per_system_cycle(SlotsPerSystemCycle::try_turbo(4).unwrap())
            .unwrap();
        for expected_slot in 1..4 {
            assert!(!clock.consume_cpu_slot());
            assert_eq!(
                clock.timestamp(),
                VirtualTimestamp {
                    system_cycle: 0,
                    slot: expected_slot,
                }
            );
        }
        assert!(clock.consume_cpu_slot());
        assert_eq!(
            clock.timestamp(),
            VirtualTimestamp {
                system_cycle: 1,
                slot: 0,
            }
        );
    }
}
