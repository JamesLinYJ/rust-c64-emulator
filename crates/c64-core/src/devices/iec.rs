// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - deterministic open-collector IEC serial bus
//
//   File:       devices/iec.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

pub const MAXIMUM_IEC_PORTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IecLine {
    Attention = 1 << 0,
    Clock = 1 << 1,
    Data = 1 << 2,
    Reset = 1 << 3,
    ServiceRequest = 1 << 4,
}

impl IecLine {
    pub const ALL: [Self; 5] = [
        Self::Attention,
        Self::Clock,
        Self::Data,
        Self::Reset,
        Self::ServiceRequest,
    ];

    pub const fn mask(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IecBusState {
    low_mask: u8,
}

impl IecBusState {
    pub const fn line_high(self, line: IecLine) -> bool {
        self.low_mask & line.mask() == 0
    }

    pub const fn attention_high(self) -> bool {
        self.line_high(IecLine::Attention)
    }

    pub const fn clock_high(self) -> bool {
        self.line_high(IecLine::Clock)
    }

    pub const fn data_high(self) -> bool {
        self.line_high(IecLine::Data)
    }

    pub const fn reset_high(self) -> bool {
        self.line_high(IecLine::Reset)
    }

    pub const fn service_request_high(self) -> bool {
        self.line_high(IecLine::ServiceRequest)
    }

    pub const fn low_mask(self) -> u8 {
        self.low_mask
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IecBusTransition {
    pub changed_mask: u8,
    pub sequence: u64,
    pub state: IecBusState,
}

impl IecBusTransition {
    pub const fn line_changed(self, line: IecLine) -> bool {
        self.changed_mask & line.mask() != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IecPort {
    slot: u8,
    generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IecBusError {
    NoFreePort,
    PortDisconnected,
}

impl fmt::Display for IecBusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFreePort => write!(formatter, "IEC bus has no free port"),
            Self::PortDisconnected => write!(formatter, "IEC port is disconnected or stale"),
        }
    }
}

impl std::error::Error for IecBusError {}

/// Allocation-free open-collector IEC fabric. A port can only pull lines low;
/// the combined state is the OR of all driver masks and is therefore independent
/// of update order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IecBus {
    driver_masks: [u8; MAXIMUM_IEC_PORTS],
    port_generations: [u32; MAXIMUM_IEC_PORTS],
    attached_mask: u8,
    aggregate_low_mask: u8,
    transition_sequence: u64,
    reset_assertion_sequence: u64,
}

impl Default for IecBus {
    fn default() -> Self {
        Self::new()
    }
}

impl IecBus {
    pub const fn new() -> Self {
        Self {
            driver_masks: [0; MAXIMUM_IEC_PORTS],
            port_generations: [0; MAXIMUM_IEC_PORTS],
            attached_mask: 0,
            aggregate_low_mask: 0,
            transition_sequence: 0,
            reset_assertion_sequence: 0,
        }
    }

    pub(crate) const fn new_with_attached_port() -> (Self, IecPort) {
        let mut bus = Self::new();
        bus.attached_mask = 1;
        bus.port_generations[0] = 1;
        (
            bus,
            IecPort {
                slot: 0,
                generation: 1,
            },
        )
    }

    pub const fn state(&self) -> IecBusState {
        IecBusState {
            low_mask: self.aggregate_low_mask,
        }
    }

    pub const fn transition_sequence(&self) -> u64 {
        self.transition_sequence
    }

    /// Number of aggregate RESET high-to-low transitions observed by the bus.
    ///
    /// Devices sample this monotonic sequence instead of only the current line
    /// level, so a complete pulse between two device clocks cannot be lost.
    pub const fn reset_assertion_sequence(&self) -> u64 {
        self.reset_assertion_sequence
    }

    pub const fn line_high(&self, line: IecLine) -> bool {
        self.aggregate_low_mask & line.mask() == 0
    }

    /// Attach one driver to the fixed-capacity bus.
    ///
    /// # Errors
    ///
    /// Returns [`IecBusError::NoFreePort`] after all slots are occupied.
    pub fn attach(&mut self) -> Result<IecPort, IecBusError> {
        for slot in 0..MAXIMUM_IEC_PORTS {
            let slot_bit = 1_u8 << slot;
            if self.attached_mask & slot_bit == 0 {
                self.attached_mask |= slot_bit;
                self.driver_masks[slot] = 0;
                self.port_generations[slot] = self.port_generations[slot].wrapping_add(1).max(1);
                return Ok(IecPort {
                    slot: slot.to_le_bytes()[0],
                    generation: self.port_generations[slot],
                });
            }
        }
        Err(IecBusError::NoFreePort)
    }

    /// Read the mask currently driven by a port.
    ///
    /// # Errors
    ///
    /// Returns [`IecBusError::PortDisconnected`] for a detached or stale handle.
    pub fn port_low_mask(&self, port: IecPort) -> Result<u8, IecBusError> {
        let slot = self.require_attached(port)?;
        Ok(self.driver_masks[slot])
    }

    /// Atomically replace every open-collector output driven by a port.
    ///
    /// # Errors
    ///
    /// Returns [`IecBusError::PortDisconnected`] for a detached or stale handle.
    pub fn set_port_low_mask(
        &mut self,
        port: IecPort,
        low_mask: u8,
    ) -> Result<Option<IecBusTransition>, IecBusError> {
        let slot = self.require_attached(port)?;
        let normalized = low_mask & Self::all_line_mask();
        if self.driver_masks[slot] == normalized {
            return Ok(None);
        }
        self.driver_masks[slot] = normalized;
        Ok(self.recompute_state())
    }

    /// Change one open-collector output without affecting the other lines.
    ///
    /// # Errors
    ///
    /// Returns [`IecBusError::PortDisconnected`] for a detached or stale handle.
    pub fn set_port_line(
        &mut self,
        port: IecPort,
        line: IecLine,
        pulled_low: bool,
    ) -> Result<Option<IecBusTransition>, IecBusError> {
        let current = self.port_low_mask(port)?;
        let next = if pulled_low {
            current | line.mask()
        } else {
            current & !line.mask()
        };
        self.set_port_low_mask(port, next)
    }

    /// Detach a driver and release every line it held low.
    ///
    /// # Errors
    ///
    /// Returns [`IecBusError::PortDisconnected`] for a detached or stale handle.
    pub fn detach(&mut self, port: IecPort) -> Result<Option<IecBusTransition>, IecBusError> {
        let slot = self.require_attached(port)?;
        self.driver_masks[slot] = 0;
        self.attached_mask &= !(1_u8 << slot);
        Ok(self.recompute_state())
    }

    const fn all_line_mask() -> u8 {
        IecLine::Attention.mask()
            | IecLine::Clock.mask()
            | IecLine::Data.mask()
            | IecLine::Reset.mask()
            | IecLine::ServiceRequest.mask()
    }

    fn require_attached(&self, port: IecPort) -> Result<usize, IecBusError> {
        let slot = usize::from(port.slot);
        if slot >= MAXIMUM_IEC_PORTS
            || self.attached_mask & (1_u8 << slot) == 0
            || self.port_generations[slot] != port.generation
        {
            return Err(IecBusError::PortDisconnected);
        }
        Ok(slot)
    }

    fn recompute_state(&mut self) -> Option<IecBusTransition> {
        let mut next = 0;
        for (slot, mask) in self.driver_masks.iter().enumerate() {
            if self.attached_mask & (1_u8 << slot) != 0 {
                next |= mask;
            }
        }
        let changed_mask = self.aggregate_low_mask ^ next;
        if changed_mask == 0 {
            return None;
        }
        self.aggregate_low_mask = next;
        self.transition_sequence = self.transition_sequence.wrapping_add(1);
        if changed_mask & IecLine::Reset.mask() != 0 && next & IecLine::Reset.mask() != 0 {
            self.reset_assertion_sequence = self.reset_assertion_sequence.wrapping_add(1);
        }
        Some(IecBusTransition {
            changed_mask,
            sequence: self.transition_sequence,
            state: self.state(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{IecBus, IecBusError, IecLine, MAXIMUM_IEC_PORTS};

    #[test]
    fn open_collector_lines_remain_low_until_every_driver_releases_them() {
        let mut bus = IecBus::new();
        let host = bus.attach().unwrap();
        let drive = bus.attach().unwrap();

        bus.set_port_line(host, IecLine::Data, true).unwrap();
        bus.set_port_line(drive, IecLine::Data, true).unwrap();
        assert!(!bus.state().data_high());
        bus.set_port_line(host, IecLine::Data, false).unwrap();
        assert!(!bus.state().data_high());
        bus.set_port_line(drive, IecLine::Data, false).unwrap();
        assert!(bus.state().data_high());
        assert_eq!(bus.transition_sequence(), 2);
    }

    #[test]
    fn detach_releases_lines_and_invalidates_stale_handles() {
        let mut bus = IecBus::new();
        let port = bus.attach().unwrap();
        bus.set_port_line(port, IecLine::Clock, true).unwrap();
        let transition = bus.detach(port).unwrap().unwrap();
        assert!(transition.line_changed(IecLine::Clock));
        assert!(bus.state().clock_high());
        assert_eq!(
            bus.set_port_line(port, IecLine::Clock, true),
            Err(IecBusError::PortDisconnected)
        );
    }

    #[test]
    fn fixed_port_table_rejects_over_subscription_without_allocation() {
        let mut bus = IecBus::new();
        for _ in 0..MAXIMUM_IEC_PORTS {
            bus.attach().unwrap();
        }
        assert_eq!(bus.attach(), Err(IecBusError::NoFreePort));
    }
}
