// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - Commodore 1541 disk-side VIA board wiring
//
//   File:       devices/drive1541/disk_via.rs
//
//   Created:    2026-08-11
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::devices::via::{Mos6522, Mos6522ControlLine, register};

use super::gcr::Drive1541SpeedZone;
use super::mechanism::{
    Drive1541ControlState, Drive1541Mechanism, Drive1541MechanismError, Drive1541StepperPhase,
};

pub const DISK_STEPPER_PHASE_MASK: u8 = 0x03;
pub const DISK_MOTOR: u8 = 1 << 2;
pub const DISK_LED: u8 = 1 << 3;
pub const DISK_WRITE_PROTECT_SENSOR: u8 = 1 << 4;
pub const DISK_SPEED_ZONE_MASK: u8 = 0x60;
pub const DISK_SYNC_NOT_FOUND: u8 = 1 << 7;

const DISK_UNUSED_INPUTS_HIGH: u8 = 0x6f;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Drive1541DiskViaError {
    InvalidDeviceNumber(u8),
    Mechanism(Drive1541MechanismError),
}

impl fmt::Display for Drive1541DiskViaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDeviceNumber(value) => {
                write!(
                    formatter,
                    "1541 IEC device number {value} is outside 8..=11"
                )
            }
            Self::Mechanism(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Drive1541DiskViaError {}

impl From<Drive1541MechanismError> for Drive1541DiskViaError {
    fn from(error: Drive1541MechanismError) -> Self {
        Self::Mechanism(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ViaOutputState {
    port_a: u8,
    port_b: u8,
    ca2_high: bool,
    cb2_high: bool,
}

impl ViaOutputState {
    const fn capture(via: &Mos6522) -> Self {
        Self {
            port_a: via.port_a_output_pins(),
            port_b: via.port_b_output_pins(),
            ca2_high: via.ca2_output_high(),
            cb2_high: via.cb2_output_high(),
        }
    }
}

/// Maps VIA2 pins to the 1541 head electronics while leaving register and
/// timer behavior in [`Mos6522`]. The mechanism is borrowed only for each
/// board transaction, so neither component owns callbacks into the other.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Drive1541DiskVia {
    via: Mos6522,
    device_number: u8,
    observed_outputs: ViaOutputState,
}

impl Drive1541DiskVia {
    /// Construct VIA2 and apply the power-on electronic state.
    ///
    /// # Errors
    ///
    /// Rejects IEC device numbers outside 8..=11.
    pub fn new(
        device_number: u8,
        mechanism: &mut Drive1541Mechanism,
    ) -> Result<Self, Drive1541DiskViaError> {
        if !(8..=11).contains(&device_number) {
            return Err(Drive1541DiskViaError::InvalidDeviceNumber(device_number));
        }
        let via = Mos6522::new();
        let mut result = Self {
            observed_outputs: ViaOutputState::capture(&via),
            via,
            device_number,
        };
        result.apply_electronic_reset_state(mechanism);
        result.synchronize_inputs(mechanism);
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

    /// Read a VIA register after sampling the current head pins.
    ///
    /// # Errors
    ///
    /// Propagates a mechanism geometry error if a control output moves the
    /// head while the access is being settled.
    pub fn read(
        &mut self,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
    ) -> Result<u8, Drive1541DiskViaError> {
        self.synchronize_and_settle(mechanism)?;
        let value = self.via.read(address);
        self.apply_changed_outputs(mechanism)?;
        if is_data_port_access(address) {
            mechanism.acknowledge_byte_ready();
        }
        self.synchronize_and_settle(mechanism)?;
        Ok(value)
    }

    /// Write a VIA register and immediately expose changed output pins.
    ///
    /// # Errors
    ///
    /// Propagates mechanism geometry and checked-arithmetic errors.
    pub fn write(
        &mut self,
        mechanism: &mut Drive1541Mechanism,
        address: u16,
        value: u8,
    ) -> Result<(), Drive1541DiskViaError> {
        self.synchronize_and_settle(mechanism)?;
        self.via.write(address, value);
        self.apply_changed_outputs(mechanism)?;
        if is_data_port_access(address) {
            mechanism.acknowledge_byte_ready();
        }
        self.synchronize_and_settle(mechanism)
    }

    /// Advance one exact 1 MHz mechanism/VIA cycle in board order.
    ///
    /// # Errors
    ///
    /// Propagates checked timing, media and mechanism errors.
    pub fn clock_cycle(
        &mut self,
        mechanism: &mut Drive1541Mechanism,
    ) -> Result<bool, Drive1541DiskViaError> {
        mechanism.tick(1)?;
        self.clock_via_cycle(mechanism)
    }

    /// Advance VIA2 only, after the owning machine has clocked the mechanism.
    ///
    /// # Errors
    ///
    /// Propagates mechanism errors raised while settling changed pins.
    pub fn clock_via_cycle(
        &mut self,
        mechanism: &mut Drive1541Mechanism,
    ) -> Result<bool, Drive1541DiskViaError> {
        self.synchronize_and_settle(mechanism)?;
        self.via.clock_cycle();
        self.apply_changed_outputs(mechanism)?;
        self.synchronize_and_settle(mechanism)?;
        Ok(self.interrupt_pending())
    }

    /// Restore VIA2 electronics without moving the physical head.
    pub fn reset(&mut self, mechanism: &mut Drive1541Mechanism) {
        self.via.reset();
        self.observed_outputs = ViaOutputState::capture(&self.via);
        mechanism.set_write_data_byte(self.via.port_a_output_pins());
        self.apply_electronic_reset_state(mechanism);
        self.synchronize_inputs(mechanism);
    }

    fn synchronize_and_settle(
        &mut self,
        mechanism: &mut Drive1541Mechanism,
    ) -> Result<(), Drive1541DiskViaError> {
        self.synchronize_inputs(mechanism);
        self.apply_changed_outputs(mechanism)
    }

    fn synchronize_inputs(&mut self, mechanism: &Drive1541Mechanism) {
        self.via.set_port_a_external_inputs(mechanism.data_byte());
        let external_port_b = DISK_UNUSED_INPUTS_HIGH
            | if mechanism.write_protect_sensor_active() {
                0
            } else {
                DISK_WRITE_PROTECT_SENSOR
            }
            | if mechanism.sync_found() {
                0
            } else {
                DISK_SYNC_NOT_FOUND
            };
        self.via.set_port_b_external_inputs(external_port_b);
        // BYTE READY is active low on the CA1 pin.
        self.via
            .signal_control_line(Mos6522ControlLine::Ca1, !mechanism.byte_ready_asserted());
    }

    fn apply_changed_outputs(
        &mut self,
        mechanism: &mut Drive1541Mechanism,
    ) -> Result<(), Drive1541DiskViaError> {
        let outputs = ViaOutputState::capture(&self.via);
        if outputs.port_a != self.observed_outputs.port_a {
            mechanism.set_write_data_byte(outputs.port_a);
        }
        if outputs.port_b != self.observed_outputs.port_b {
            mechanism.apply_control_state(Drive1541ControlState {
                led_on: outputs.port_b & DISK_LED != 0,
                motor_on: outputs.port_b & DISK_MOTOR != 0,
                speed_zone: Drive1541SpeedZone::try_from(
                    (outputs.port_b & DISK_SPEED_ZONE_MASK) >> 5,
                )
                .map_err(Drive1541MechanismError::from)?,
                stepper_phase: Drive1541StepperPhase::try_from(
                    outputs.port_b & DISK_STEPPER_PHASE_MASK,
                )?,
            })?;
        }
        if outputs.ca2_high != self.observed_outputs.ca2_high {
            mechanism.set_byte_ready_enabled(outputs.ca2_high);
        }
        if outputs.cb2_high != self.observed_outputs.cb2_high {
            mechanism.set_read_mode(outputs.cb2_high);
        }
        self.observed_outputs = outputs;
        Ok(())
    }

    fn apply_electronic_reset_state(&mut self, mechanism: &mut Drive1541Mechanism) {
        mechanism.set_motor_on(false);
        mechanism.set_led_on(true);
        mechanism.set_speed_zone(Drive1541SpeedZone::Zone0);
        mechanism.set_read_mode(true);
        mechanism.set_byte_ready_enabled(true);
        mechanism.acknowledge_byte_ready();
        self.via.signal_control_line(Mos6522ControlLine::Ca1, true);
    }
}

fn is_data_port_access(address: u16) -> bool {
    matches!(
        address.to_le_bytes()[0] & 0x0f,
        register::PORT_A | register::PORT_B | register::PORT_A_WITHOUT_HANDSHAKE
    )
}
