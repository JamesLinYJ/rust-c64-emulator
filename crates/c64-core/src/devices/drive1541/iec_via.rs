// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 IEC-side VIA board wiring
//
//   File:       devices/drive1541/iec_via.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::devices::iec::{IecBus, IecBusError, IecLine, IecPort};
use crate::devices::via::{Mos6522, Mos6522ControlLine};

pub const IEC_DATA_INPUT: u8 = 1 << 0;
pub const IEC_DATA_OUTPUT: u8 = 1 << 1;
pub const IEC_CLOCK_INPUT: u8 = 1 << 2;
pub const IEC_CLOCK_OUTPUT: u8 = 1 << 3;
pub const IEC_ATTENTION_ACKNOWLEDGE_OUTPUT: u8 = 1 << 4;
pub const IEC_DEVICE_ADDRESS_MASK: u8 = 0x60;
pub const IEC_ATTENTION_INPUT: u8 = 1 << 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Drive1541IecViaError {
    InvalidDeviceNumber(u8),
    Bus(IecBusError),
}

impl fmt::Display for Drive1541IecViaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDeviceNumber(value) => {
                write!(
                    formatter,
                    "1541 IEC device number {value} is outside 8..=11"
                )
            }
            Self::Bus(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Drive1541IecViaError {}

impl From<IecBusError> for Drive1541IecViaError {
    fn from(error: IecBusError) -> Self {
        Self::Bus(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct Drive1541IecVia {
    via: Mos6522,
    device_number: u8,
    bus_port: IecPort,
}

impl Drive1541IecVia {
    /// Attach a VIA1 board interface to an IEC bus.
    ///
    /// # Errors
    ///
    /// Rejects device numbers outside 8..=11 or a bus with no free port.
    pub fn new(device_number: u8, bus: &mut IecBus) -> Result<Self, Drive1541IecViaError> {
        if !(8..=11).contains(&device_number) {
            return Err(Drive1541IecViaError::InvalidDeviceNumber(device_number));
        }
        let bus_port = bus.attach()?;
        let mut result = Self {
            via: Mos6522::new(),
            device_number,
            bus_port,
        };
        result.synchronize_inputs(bus);
        result.update_outputs(bus);
        Ok(result)
    }

    pub const fn device_number(&self) -> u8 {
        self.device_number
    }

    pub const fn via(&self) -> &Mos6522 {
        &self.via
    }

    pub const fn interrupt_pending(&self) -> bool {
        self.via.interrupt_pending()
    }

    pub fn read(&mut self, bus: &mut IecBus, address: u16) -> u8 {
        self.synchronize_inputs(bus);
        let value = self.via.read(address);
        self.update_outputs(bus);
        value
    }

    pub fn write(&mut self, bus: &mut IecBus, address: u16, value: u8) {
        self.synchronize_inputs(bus);
        self.via.write(address, value);
        self.update_outputs(bus);
    }

    pub fn clock_cycle(&mut self, bus: &mut IecBus) -> bool {
        self.synchronize_inputs(bus);
        self.via.clock_cycle();
        self.update_outputs(bus);
        self.interrupt_pending()
    }

    pub fn reset(&mut self, bus: &mut IecBus) {
        self.via.reset();
        self.synchronize_inputs(bus);
        self.update_outputs(bus);
    }

    /// Release every line and detach this VIA from the shared bus.
    ///
    /// # Errors
    ///
    /// Returns a bus error only if the port was already detached.
    pub fn disconnect(self, bus: &mut IecBus) -> Result<(), Drive1541IecViaError> {
        bus.detach(self.bus_port)?;
        Ok(())
    }

    fn synchronize_inputs(&mut self, bus: &IecBus) {
        let state = bus.state();
        let address_switches = (self.device_number - 8) << 5;
        let external_port_b = IEC_DATA_OUTPUT
            | IEC_CLOCK_OUTPUT
            | IEC_ATTENTION_ACKNOWLEDGE_OUTPUT
            | address_switches
            | if state.data_high() { 0 } else { IEC_DATA_INPUT }
            | if state.clock_high() {
                0
            } else {
                IEC_CLOCK_INPUT
            }
            | if state.attention_high() {
                0
            } else {
                IEC_ATTENTION_INPUT
            };
        self.via.set_port_b_external_inputs(external_port_b);
        // The board inverter makes asserted-low IEC ATN a rising CA1 edge.
        self.via
            .signal_control_line(Mos6522ControlLine::Ca1, !state.attention_high());
    }

    fn update_outputs(&self, bus: &mut IecBus) {
        let output_latch = self.via.port_b_output_latch();
        let output_direction = self.via.port_b_data_direction();
        let asserted = output_latch & output_direction;
        let mut low_mask = 0;
        if asserted & IEC_CLOCK_OUTPUT != 0 {
            low_mask |= IecLine::Clock.mask();
        }
        let data_output_asserted = asserted & IEC_DATA_OUTPUT != 0;
        let attention_acknowledge_is_output =
            output_direction & IEC_ATTENTION_ACKNOWLEDGE_OUTPUT != 0;
        let attention_acknowledge_high = output_latch & IEC_ATTENTION_ACKNOWLEDGE_OUTPUT != 0;
        let attention_gate_asserted = attention_acknowledge_is_output
            && attention_acknowledge_high == bus.state().attention_high();
        if data_output_asserted || attention_gate_asserted {
            low_mask |= IecLine::Data.mask();
        }
        let result = bus.set_port_low_mask(self.bus_port, low_mask);
        debug_assert!(result.is_ok(), "the 1541 IEC VIA port must stay attached");
    }
}

#[cfg(test)]
mod tests {
    use crate::devices::iec::{IecBus, IecLine};
    use crate::devices::via::{interrupt, register};

    use super::{
        Drive1541IecVia, IEC_ATTENTION_ACKNOWLEDGE_OUTPUT, IEC_CLOCK_OUTPUT, IEC_DATA_OUTPUT,
    };

    #[test]
    fn board_inverters_map_bus_levels_and_address_switches_to_port_b() {
        let mut bus = IecBus::new();
        let host = bus.attach().unwrap();
        let mut via = Drive1541IecVia::new(10, &mut bus).unwrap();
        bus.set_port_low_mask(host, IecLine::Clock.mask() | IecLine::Data.mask())
            .unwrap();

        assert_eq!(via.read(&mut bus, u16::from(register::PORT_B)), 0x5f);
    }

    #[test]
    fn high_via_outputs_pull_inverted_iec_clock_and_data_low() {
        let mut bus = IecBus::new();
        let mut via = Drive1541IecVia::new(8, &mut bus).unwrap();
        via.write(
            &mut bus,
            u16::from(register::DATA_DIRECTION_B),
            IEC_CLOCK_OUTPUT | IEC_DATA_OUTPUT,
        );
        via.write(
            &mut bus,
            u16::from(register::PORT_B),
            IEC_CLOCK_OUTPUT | IEC_DATA_OUTPUT,
        );
        assert!(!bus.state().clock_high());
        assert!(!bus.state().data_high());

        via.write(&mut bus, u16::from(register::PORT_B), 0);
        assert!(bus.state().clock_high());
        assert!(bus.state().data_high());
    }

    #[test]
    fn attention_inverter_produces_the_rom_configured_positive_ca1_edge() {
        let mut bus = IecBus::new();
        let host = bus.attach().unwrap();
        let mut via = Drive1541IecVia::new(8, &mut bus).unwrap();
        via.write(&mut bus, u16::from(register::PERIPHERAL_CONTROL), 0x01);
        bus.set_port_line(host, IecLine::Attention, true).unwrap();
        via.clock_cycle(&mut bus);
        assert_ne!(via.via().interrupt_flags() & interrupt::CA1, 0);
    }

    #[test]
    fn atna_gate_acknowledges_matching_attention_level() {
        let mut bus = IecBus::new();
        let mut via = Drive1541IecVia::new(8, &mut bus).unwrap();
        via.write(
            &mut bus,
            u16::from(register::DATA_DIRECTION_B),
            IEC_ATTENTION_ACKNOWLEDGE_OUTPUT,
        );
        via.write(
            &mut bus,
            u16::from(register::PORT_B),
            IEC_ATTENTION_ACKNOWLEDGE_OUTPUT,
        );
        assert!(!bus.state().data_high());
    }
}
