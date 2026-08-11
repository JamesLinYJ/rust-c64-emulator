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

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    wincode::SchemaRead,
    wincode::SchemaWrite,
)]
pub struct VirtualTimestamp {
    pub system_cycle: u64,
    pub slot: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
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

    /// Return the next boundary at which C64 devices must be advanced.
    ///
    /// A slot-zero timestamp has not serviced that boundary yet, so it is the
    /// event itself. Once an internal Turbo slot has retired, the next event is
    /// slot zero of the following system cycle.
    pub const fn next_external_event(self) -> VirtualTimestamp {
        if self.timestamp.slot == 0 {
            self.timestamp
        } else {
            VirtualTimestamp {
                system_cycle: self.timestamp.system_cycle.wrapping_add(1),
                slot: 0,
            }
        }
    }

    /// Number of CPU slots a fast executor may retire without crossing the next
    /// external-device event. Zero means the scheduler must service hardware
    /// before executing another CPU slot.
    pub const fn cpu_slots_before_next_external_event(self) -> u8 {
        if self.timestamp.slot == 0 {
            0
        } else {
            self.slots_per_system_cycle.get() - self.timestamp.slot
        }
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

    pub(crate) fn consume_strict_cpu_slot(&mut self) {
        debug_assert_eq!(self.timestamp.slot, 0);
        debug_assert_eq!(self.slots_per_system_cycle, SlotsPerSystemCycle::STRICT);
        self.timestamp.system_cycle = self.timestamp.system_cycle.wrapping_add(1);
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

    #[test]
    fn turbo_event_horizon_never_crosses_an_external_system_cycle() {
        let mut clock = VirtualClock::new();
        clock
            .set_slots_per_system_cycle(SlotsPerSystemCycle::try_turbo(8).unwrap())
            .unwrap();

        assert_eq!(clock.cpu_slots_before_next_external_event(), 0);
        assert_eq!(clock.next_external_event(), clock.timestamp());

        assert!(!clock.consume_cpu_slot());
        assert_eq!(clock.cpu_slots_before_next_external_event(), 7);
        assert_eq!(
            clock.next_external_event(),
            VirtualTimestamp {
                system_cycle: 1,
                slot: 0,
            }
        );
    }
}
