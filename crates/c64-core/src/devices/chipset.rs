// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - C64 主板级芯片接线
//
//   文件:       chipset.rs
//
//   日期:       2026年08月11日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::address_space::{C64BusDevices, CartridgeLines};
use crate::architecture::VideoStandard;
use crate::processor_port::{ProcessorPortInputState, ProcessorPortOutputState};

use super::cartridge::{Cartridge, CartridgeError};
use super::cia::{Mos6526, Mos6526Model, Mos6526Timing};
use super::drive1541::drive::{Commodore1541Drive, Commodore1541DriveError};
use super::iec::{IecBus, IecLine, IecPort};
use super::input::{
    C64HostInput, C64HostInputError, C64HostInputPortState, C64HostInputPortValues,
};
use super::reu::{RamExpansionUnit, ReuImageError};
use super::sid::{
    DEFAULT_SAMPLE_RATE_HZ, NTSC_PROCESSOR_CLOCK_HZ, PAL_PROCESSOR_CLOCK_HZ, Sid, SidModel,
};
use super::tape::{Commodore1530Datasette, DatasetteError, DatasetteHostSignals};
use super::vic::{NTSC_VIC_TIMING, PAL_VIC_TIMING, VicError, VicII, VicMemoryBus};

const CIA1_PORT_B_LIGHT_PEN_INPUT: u8 = 1 << 4;
const PROCESSOR_PORT_CASSETTE_WRITE: u8 = 1 << 3;
const PROCESSOR_PORT_CASSETTE_SENSE: u8 = 1 << 4;
const PROCESSOR_PORT_CASSETTE_MOTOR: u8 = 1 << 5;
const CIA2_IEC_ATTENTION_OUTPUT: u8 = 1 << 3;
const CIA2_IEC_CLOCK_OUTPUT: u8 = 1 << 4;
const CIA2_IEC_DATA_OUTPUT: u8 = 1 << 5;
const CIA2_IEC_CLOCK_INPUT: u8 = 1 << 6;
const CIA2_IEC_DATA_INPUT: u8 = 1 << 7;
const CIA2_NON_IEC_INPUTS_HIGH: u8 = 0x3f;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum C64ChipsetError {
    Cartridge(CartridgeError),
    CartridgeAlreadyAttached,
    CartridgeNotAttached,
    EasyFlashNotAttached,
    ReuImage(ReuImageError),
    ReuAlreadyAttached,
    ReuNotAttached,
    Datasette(DatasetteError),
    Drive1541(Commodore1541DriveError),
    Drive1541AlreadyAttached,
    Drive1541NotAttached,
    HostInput(C64HostInputError),
    Vic(VicError),
}

impl fmt::Display for C64ChipsetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cartridge(error) => error.fmt(formatter),
            Self::CartridgeAlreadyAttached => {
                formatter.write_str("a cartridge is already attached")
            }
            Self::CartridgeNotAttached => formatter.write_str("no cartridge is attached"),
            Self::EasyFlashNotAttached => {
                formatter.write_str("the attached cartridge is not an EasyFlash board")
            }
            Self::ReuImage(error) => error.fmt(formatter),
            Self::ReuAlreadyAttached => {
                formatter.write_str("the expansion port is already occupied")
            }
            Self::ReuNotAttached => formatter.write_str("no REU is attached"),
            Self::Datasette(error) => error.fmt(formatter),
            Self::Drive1541(error) => error.fmt(formatter),
            Self::Drive1541AlreadyAttached => {
                formatter.write_str("a Commodore 1541 is already attached")
            }
            Self::Drive1541NotAttached => formatter.write_str("no Commodore 1541 is attached"),
            Self::HostInput(error) => error.fmt(formatter),
            Self::Vic(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for C64ChipsetError {}

impl From<CartridgeError> for C64ChipsetError {
    fn from(error: CartridgeError) -> Self {
        Self::Cartridge(error)
    }
}

impl From<DatasetteError> for C64ChipsetError {
    fn from(error: DatasetteError) -> Self {
        Self::Datasette(error)
    }
}

impl From<ReuImageError> for C64ChipsetError {
    fn from(error: ReuImageError) -> Self {
        Self::ReuImage(error)
    }
}

impl From<Commodore1541DriveError> for C64ChipsetError {
    fn from(error: Commodore1541DriveError) -> Self {
        Self::Drive1541(error)
    }
}

impl From<C64HostInputError> for C64ChipsetError {
    fn from(error: C64HostInputError) -> Self {
        Self::HostInput(error)
    }
}

impl From<VicError> for C64ChipsetError {
    fn from(error: VicError) -> Self {
        Self::Vic(error)
    }
}

/// 主板级芯片接线。尚未迁移的扩展范围保持明确 open bus，不存在 TypeScript fallback。
#[derive(Clone, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct C64Chipset {
    video_standard: VideoStandard,
    irq_cia: Mos6526,
    nmi_cia: Mos6526,
    vic: VicII,
    sid: Sid,
    iec_bus: IecBus,
    iec_host_port: IecPort,
    iec_reset_asserted: bool,
    drive1541: Option<Commodore1541Drive>,
    datasette: Commodore1530Datasette,
    pending_datasette_error: Option<DatasetteError>,
    tape_read_line_high: bool,
    cartridge: Option<Cartridge>,
    reu: Option<RamExpansionUnit>,
    #[wincode(skip(default_val = C64HostInput::new()))]
    host_input: C64HostInput,
    processor_port_output: ProcessorPortOutputState,
    last_cpu_read_was_held: bool,
}

impl C64Chipset {
    pub fn new(video_standard: VideoStandard) -> Self {
        Self::new_with_sid_model(video_standard, SidModel::Mos6581)
    }

    pub fn new_with_sid_model(video_standard: VideoStandard, sid_model: SidModel) -> Self {
        let cia_timing = match video_standard {
            VideoStandard::Pal => Mos6526Timing::PAL,
            VideoStandard::Ntsc => Mos6526Timing::NTSC,
        };
        let vic_timing = match video_standard {
            VideoStandard::Pal => PAL_VIC_TIMING,
            VideoStandard::Ntsc => NTSC_VIC_TIMING,
        };
        let sid_clock_hz = match video_standard {
            VideoStandard::Pal => PAL_PROCESSOR_CLOCK_HZ,
            VideoStandard::Ntsc => NTSC_PROCESSOR_CLOCK_HZ,
        };
        let (iec_bus, iec_host_port) = IecBus::new_with_attached_port();
        Self {
            video_standard,
            irq_cia: Mos6526::new_with_valid_timing(Mos6526Model::Original, cia_timing),
            nmi_cia: Mos6526::new_with_valid_timing(Mos6526Model::Original, cia_timing),
            vic: VicII::new_with_timing(vic_timing),
            sid: Sid::new_with_valid_rates(sid_model, sid_clock_hz, DEFAULT_SAMPLE_RATE_HZ),
            iec_bus,
            iec_host_port,
            iec_reset_asserted: false,
            drive1541: None,
            datasette: Commodore1530Datasette::new_with_valid_target_clock(
                video_standard.system_clock_hz(),
            ),
            pending_datasette_error: None,
            tape_read_line_high: true,
            cartridge: None,
            reu: None,
            host_input: C64HostInput::new(),
            processor_port_output: ProcessorPortOutputState {
                direction: 0,
                output_latch: 0,
                output_pins: 0xff,
            },
            last_cpu_read_was_held: false,
        }
    }

    pub const fn cia1(&self) -> &Mos6526 {
        &self.irq_cia
    }

    pub const fn cia2(&self) -> &Mos6526 {
        &self.nmi_cia
    }

    pub const fn cia1_mut(&mut self) -> &mut Mos6526 {
        &mut self.irq_cia
    }

    pub const fn cia2_mut(&mut self) -> &mut Mos6526 {
        &mut self.nmi_cia
    }

    pub fn irq_asserted(&self) -> bool {
        self.irq_cia.interrupt_pending()
            || self.vic.interrupt_pending()
            || self.cartridge.as_ref().is_some_and(Cartridge::irq_line_low)
            || self
                .reu
                .as_ref()
                .is_some_and(RamExpansionUnit::irq_line_low)
    }

    pub fn nmi_asserted(&self) -> bool {
        self.nmi_cia.interrupt_pending()
            || self.cartridge.as_ref().is_some_and(Cartridge::nmi_line_low)
            || self.host_input.nmi_asserted()
    }

    pub const fn processor_port_output(&self) -> ProcessorPortOutputState {
        self.processor_port_output
    }

    pub const fn vic(&self) -> &VicII {
        &self.vic
    }

    pub const fn vic_mut(&mut self) -> &mut VicII {
        &mut self.vic
    }

    pub const fn sid(&self) -> &Sid {
        &self.sid
    }

    pub const fn sid_mut(&mut self) -> &mut Sid {
        &mut self.sid
    }

    pub const fn iec_bus(&self) -> &IecBus {
        &self.iec_bus
    }

    pub const fn iec_bus_mut(&mut self) -> &mut IecBus {
        &mut self.iec_bus
    }

    pub const fn drive1541(&self) -> Option<&Commodore1541Drive> {
        self.drive1541.as_ref()
    }

    pub const fn drive1541_mut(&mut self) -> Option<&mut Commodore1541Drive> {
        self.drive1541.as_mut()
    }

    pub const fn datasette(&self) -> &Commodore1530Datasette {
        &self.datasette
    }

    pub const fn datasette_mut(&mut self) -> &mut Commodore1530Datasette {
        &mut self.datasette
    }

    pub const fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    pub const fn cartridge_mut(&mut self) -> Option<&mut Cartridge> {
        self.cartridge.as_mut()
    }

    pub const fn reu(&self) -> Option<&RamExpansionUnit> {
        self.reu.as_ref()
    }

    pub const fn reu_mut(&mut self) -> Option<&mut RamExpansionUnit> {
        self.reu.as_mut()
    }

    pub const fn host_input(&self) -> &C64HostInput {
        &self.host_input
    }

    /// 原子替换浏览器或原生前端提供的键盘、操纵杆与 RESTORE 快照。
    ///
    /// # Errors
    ///
    /// 键盘列数或操纵杆数字线掩码无效时拒绝整个快照。
    pub fn set_host_input(
        &mut self,
        pressed_rows_by_column: &[u8],
        shift_lock_pressed: bool,
        joystick_port_1_grounded: u8,
        joystick_port_2_grounded: u8,
        restore_key_pressed: bool,
    ) -> Result<(), C64ChipsetError> {
        self.host_input.set_state(
            pressed_rows_by_column,
            shift_lock_pressed,
            joystick_port_1_grounded,
            joystick_port_2_grounded,
            restore_key_pressed,
        )?;
        self.synchronize_light_pen_input();
        Ok(())
    }

    pub(crate) fn state_matches_config(
        &self,
        video_standard: VideoStandard,
        sid_model: SidModel,
    ) -> bool {
        let vic_timing = match video_standard {
            VideoStandard::Pal => PAL_VIC_TIMING,
            VideoStandard::Ntsc => NTSC_VIC_TIMING,
        };
        self.video_standard == video_standard
            && self.vic.state_matches_timing(vic_timing)
            && self.sid.model() == sid_model
            && self.sid.processor_clock_hz() == video_standard.system_clock_hz()
            && self.sid.state_is_valid()
            && self.datasette.target_clock_hz() == video_standard.system_clock_hz()
            && self
                .reu
                .as_ref()
                .is_none_or(RamExpansionUnit::state_is_valid)
    }

    /// Attach one fully validated cartridge to the expansion port.
    ///
    /// # Errors
    ///
    /// Rejects attachment while the physical slot is occupied.
    pub fn attach_cartridge(&mut self, cartridge: Cartridge) -> Result<(), C64ChipsetError> {
        if self.cartridge.is_some() || self.reu.is_some() {
            return Err(C64ChipsetError::CartridgeAlreadyAttached);
        }
        self.cartridge = Some(cartridge);
        Ok(())
    }

    /// Detach and return the current expansion-port cartridge.
    ///
    /// # Errors
    ///
    /// Rejects an empty physical slot.
    pub fn detach_cartridge(&mut self) -> Result<Cartridge, C64ChipsetError> {
        self.cartridge
            .take()
            .ok_or(C64ChipsetError::CartridgeNotAttached)
    }

    /// Attach one 17xx REU to the physical expansion port.
    ///
    /// # Errors
    ///
    /// Rejects attachment while the expansion port is occupied.
    pub fn attach_reu(&mut self, reu: RamExpansionUnit) -> Result<(), C64ChipsetError> {
        if self.reu.is_some() || self.cartridge.is_some() {
            return Err(C64ChipsetError::ReuAlreadyAttached);
        }
        self.reu = Some(reu);
        Ok(())
    }

    /// Detach and return the current 17xx REU.
    ///
    /// # Errors
    ///
    /// Rejects an expansion port without a REU.
    pub fn detach_reu(&mut self) -> Result<RamExpansionUnit, C64ChipsetError> {
        self.reu.take().ok_or(C64ChipsetError::ReuNotAttached)
    }

    /// Attach one explicitly configured 1541 to this board's shared IEC bus.
    ///
    /// # Errors
    ///
    /// Rejects duplicate attachment and propagates drive construction errors.
    pub fn attach_drive1541(
        &mut self,
        device_number: u8,
        rom: &[u8],
    ) -> Result<(), C64ChipsetError> {
        if self.drive1541.is_some() {
            return Err(C64ChipsetError::Drive1541AlreadyAttached);
        }
        let drive =
            Commodore1541Drive::new(device_number, rom, self.video_standard, &mut self.iec_bus)?;
        self.drive1541 = Some(drive);
        Ok(())
    }

    /// Detach the configured 1541 and release its IEC port.
    ///
    /// # Errors
    ///
    /// Rejects an empty drive slot and propagates IEC detach errors.
    pub fn detach_drive1541(&mut self) -> Result<(), C64ChipsetError> {
        let drive = self
            .drive1541
            .take()
            .ok_or(C64ChipsetError::Drive1541NotAttached)?;
        drive.disconnect(&mut self.iec_bus)?;
        Ok(())
    }

    pub const fn ba_low(&self) -> bool {
        self.vic.ba_low()
    }

    pub const fn aec_low(&self) -> bool {
        self.vic.aec_low()
    }

    pub fn begin_cpu_read(&mut self) {
        self.last_cpu_read_was_held = false;
    }

    pub fn mark_cpu_read_held(&mut self) {
        self.last_cpu_read_was_held = true;
    }

    /// # Errors
    ///
    /// VIC 半周期计划或 1541 调度出现内部不一致时返回显式错误。
    pub fn clock_system_cycle<M: VicMemoryBus>(
        &mut self,
        memory: &mut M,
    ) -> Result<(), C64ChipsetError> {
        let datasette_result = self.clock_datasette();
        self.clock_cartridge();
        self.clock_cias();
        let vic_result = self.clock_vic(memory);
        self.clock_sid();
        let drive_result = self.clock_drive1541();
        self.clock_host_input();
        datasette_result?;
        vic_result?;
        drive_result
    }

    pub(crate) fn clock_datasette(&mut self) -> Result<(), C64ChipsetError> {
        if let Some(error) = self.pending_datasette_error.take() {
            return Err(error.into());
        }
        let result = self.datasette.clock_cycle()?;
        for _ in 0..result.read_pulses {
            self.pulse_tape_read_line();
        }
        Ok(())
    }

    pub(crate) fn clock_cias(&mut self) {
        self.irq_cia.clock_cycle();
        self.nmi_cia.clock_cycle();
        self.synchronize_light_pen_input();
    }

    pub(crate) fn clock_cartridge(&mut self) {
        if let Some(cartridge) = self.cartridge.as_mut() {
            cartridge.clock_cycles(1);
        }
    }

    pub(crate) fn clock_vic<M: VicMemoryBus>(&mut self, memory: &mut M) -> Result<(), VicError> {
        self.vic.clock_cycle(memory).map(|_| ())
    }

    pub(crate) fn clock_sid(&mut self) {
        self.sid.clock_cycle();
    }

    pub fn clock_host_input(&mut self) {
        self.host_input.clock_cycle();
    }

    pub(crate) fn clock_drive1541(&mut self) -> Result<(), C64ChipsetError> {
        let result = if let Some(drive) = self.drive1541.as_mut() {
            drive.clock_host_cycle(&mut self.iec_bus).map(|_| ())
        } else {
            Ok(())
        };
        self.synchronize_cia1_flag_input();
        result.map_err(Into::into)
    }

    /// Pulse the board RESET line and reset every attached chip and drive.
    ///
    /// # Errors
    ///
    /// Propagates a drive error after always releasing the IEC RESET line.
    pub fn reset(&mut self) -> Result<(), C64ChipsetError> {
        let drive_reset_result = self.set_iec_reset_asserted(true);
        self.irq_cia.reset();
        self.nmi_cia.reset();
        self.vic.reset();
        self.sid.reset();
        if let Some(cartridge) = self.cartridge.as_mut() {
            cartridge.reset();
        }
        if let Some(reu) = self.reu.as_mut() {
            reu.reset();
        }
        self.host_input.reset_restore_circuit();
        let release_result = self.set_iec_reset_asserted(false);
        self.synchronize_light_pen_input();
        self.synchronize_cia1_flag_input();
        self.last_cpu_read_was_held = false;
        drive_reset_result?;
        release_result
    }

    pub(crate) fn reinitialize_host(
        &mut self,
        video_standard: VideoStandard,
        sid_model: SidModel,
    ) -> Result<(), C64ChipsetError> {
        let fresh = Self::new_with_sid_model(video_standard, sid_model);
        self.video_standard = fresh.video_standard;
        self.irq_cia = fresh.irq_cia;
        self.nmi_cia = fresh.nmi_cia;
        self.vic = fresh.vic;
        self.sid = fresh.sid;
        self.processor_port_output = fresh.processor_port_output;
        self.pending_datasette_error = None;
        self.tape_read_line_high = true;
        self.last_cpu_read_was_held = false;
        self.iec_reset_asserted = false;
        self.update_iec_host_outputs();
        if let Some(drive) = self.drive1541.as_mut() {
            drive.reconfigure_host_clock(video_standard);
        }
        self.datasette
            .reconfigure_target_clock(video_standard.system_clock_hz())?;
        self.datasette
            .set_host_signals(Self::datasette_host_signals(self.processor_port_output))?;
        self.reset()
    }

    pub fn vic_bank_address(&self) -> u16 {
        u16::from(!self.nmi_cia.port_a_output_pins() & 0x03) << 14
    }

    fn synchronize_light_pen_input(&mut self) {
        let inputs = self.cia1_port_inputs();
        self.vic
            .set_light_pen_input_high(inputs.port_b & CIA1_PORT_B_LIGHT_PEN_INPUT != 0);
    }

    fn cia1_port_inputs(&self) -> C64HostInputPortValues {
        self.host_input.resolve_port_inputs(
            C64HostInputPortState {
                data_direction: self.irq_cia.port_a_data_direction(),
                external_input_pins: !self.host_input.joystick_port_2_grounded(),
                output_pins: self.irq_cia.port_a_output_pins(),
            },
            C64HostInputPortState {
                data_direction: self.irq_cia.port_b_data_direction(),
                external_input_pins: !self.host_input.joystick_port_1_grounded(),
                output_pins: self.irq_cia.port_b_output_pins(),
            },
        )
    }

    fn pulse_tape_read_line(&mut self) {
        self.tape_read_line_high = false;
        self.synchronize_cia1_flag_input();
        self.tape_read_line_high = true;
        self.synchronize_cia1_flag_input();
    }

    fn synchronize_cia1_flag_input(&mut self) {
        self.irq_cia.set_flag_pin_high(
            self.tape_read_line_high && self.iec_bus.state().service_request_high(),
        );
    }

    const fn datasette_host_signals(state: ProcessorPortOutputState) -> DatasetteHostSignals {
        DatasetteHostSignals {
            motor_active: state.output_pins & PROCESSOR_PORT_CASSETTE_MOTOR == 0,
            write_high: state.output_pins & PROCESSOR_PORT_CASSETTE_WRITE != 0,
        }
    }

    fn cia2_port_a_external_inputs(&self) -> u8 {
        let state = self.iec_bus.state();
        CIA2_NON_IEC_INPUTS_HIGH
            | if state.clock_high() {
                CIA2_IEC_CLOCK_INPUT
            } else {
                0
            }
            | if state.data_high() {
                CIA2_IEC_DATA_INPUT
            } else {
                0
            }
    }

    fn set_iec_reset_asserted(&mut self, asserted: bool) -> Result<(), C64ChipsetError> {
        if self.iec_reset_asserted == asserted {
            return Ok(());
        }
        self.iec_reset_asserted = asserted;
        self.update_iec_host_outputs();
        if asserted && let Some(drive) = self.drive1541.as_mut() {
            drive.synchronize_iec_reset(&mut self.iec_bus)?;
        }
        Ok(())
    }

    fn update_iec_host_outputs(&mut self) {
        let asserted = self.nmi_cia.port_a_output_latch() & self.nmi_cia.port_a_data_direction();
        let mut low_mask = 0;
        if asserted & CIA2_IEC_ATTENTION_OUTPUT != 0 {
            low_mask |= IecLine::Attention.mask();
        }
        if asserted & CIA2_IEC_CLOCK_OUTPUT != 0 {
            low_mask |= IecLine::Clock.mask();
        }
        if asserted & CIA2_IEC_DATA_OUTPUT != 0 {
            low_mask |= IecLine::Data.mask();
        }
        if self.iec_reset_asserted {
            low_mask |= IecLine::Reset.mask();
        }
        let result = self.iec_bus.set_port_low_mask(self.iec_host_port, low_mask);
        debug_assert!(result.is_ok(), "the C64 IEC host port must stay attached");
    }
}

impl Default for C64Chipset {
    fn default() -> Self {
        Self::new(VideoStandard::Pal)
    }
}

impl C64BusDevices for C64Chipset {
    fn cartridge_lines(&self) -> CartridgeLines {
        self.cartridge
            .as_ref()
            .map_or(CartridgeLines::DISCONNECTED, Cartridge::lines)
    }

    fn read_cartridge(
        &mut self,
        region: crate::address_space::CartridgeRegion,
        address: u16,
    ) -> Option<u8> {
        if region == crate::address_space::CartridgeRegion::Io2
            && let Some(reu) = self.reu.as_mut()
        {
            return reu.read_io2(address);
        }
        self.cartridge
            .as_mut()
            .and_then(|cartridge| cartridge.read(region, address))
    }

    fn write_cartridge(
        &mut self,
        region: crate::address_space::CartridgeRegion,
        address: u16,
        value: u8,
    ) {
        if region == crate::address_space::CartridgeRegion::Io2
            && let Some(reu) = self.reu.as_mut()
        {
            reu.write_io2(address, value);
            return;
        }
        if let Some(cartridge) = self.cartridge.as_mut() {
            cartridge.write(region, address, value);
        }
    }

    fn read_io(&mut self, address: u16, open_bus: u8) -> u8 {
        match address >> 8 {
            0xd0..=0xd3 => self.vic.read_register(address),
            0xd4..=0xd7 => self.sid.read(address),
            0xdc => {
                let inputs = self.cia1_port_inputs();
                self.irq_cia.read(address, inputs.port_a, inputs.port_b)
            }
            0xdd => self
                .nmi_cia
                .read(address, self.cia2_port_a_external_inputs(), 0xff),
            _ => open_bus,
        }
    }

    fn write_io(&mut self, address: u16, value: u8) {
        match address >> 8 {
            0xd0..=0xd3 => self.vic.write_register(address, value),
            0xd4..=0xd7 => self.sid.write(address, value),
            0xdc => self.irq_cia.write(address, value),
            0xdd => {
                self.nmi_cia.write(address, value);
                self.update_iec_host_outputs();
            }
            _ => {}
        }
    }

    fn cpu_read_was_held(&self) -> bool {
        self.last_cpu_read_was_held
    }

    fn processor_port_input_state(&self) -> ProcessorPortInputState {
        ProcessorPortInputState {
            mask: PROCESSOR_PORT_CASSETTE_SENSE,
            value: if self.datasette.sense_switch_closed() {
                0
            } else {
                PROCESSOR_PORT_CASSETTE_SENSE
            },
        }
    }

    fn open_bus_value(&self) -> u8 {
        self.vic.phi1_data_bus_value()
    }

    fn processor_port_output_changed(&mut self, state: ProcessorPortOutputState) {
        self.processor_port_output = state;
        if let Err(error) = self
            .datasette
            .set_host_signals(Self::datasette_host_signals(state))
        {
            self.pending_datasette_error.get_or_insert(error);
        }
    }

    fn observe_cpu_write(&mut self, address: u16) {
        if let Some(reu) = self.reu.as_mut() {
            reu.observe_cpu_write(address);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::address_space::C64BusDevices;
    use crate::architecture::VideoStandard;
    use crate::devices::cia::{control, interrupt, register};
    use crate::devices::iec::IecLine;
    use crate::devices::sid::{control as sid_control, register as sid_register};
    use crate::devices::vic::VicMemoryBus;

    use super::{C64Chipset, CIA1_PORT_B_LIGHT_PEN_INPUT};

    struct ZeroVicMemory;

    impl VicMemoryBus for ZeroVicMemory {
        fn cpu_data_bus_value(&self) -> u8 {
            0xff
        }

        fn read_vic_byte(&mut self, _address_in_bank: u16) -> u8 {
            0
        }

        fn read_vic_color(&mut self, _index: u16) -> u8 {
            0
        }
    }

    #[test]
    fn board_io_routes_dc_to_irq_cia_and_dd_to_nmi_cia() {
        let mut devices = C64Chipset::new(VideoStandard::Pal);
        let mut memory = ZeroVicMemory;
        devices.write_io(0xdc04, 1);
        devices.write_io(0xdc05, 0);
        devices.write_io(0xdc0d, interrupt::SET_OR_PENDING | interrupt::TIMER_A);
        devices.write_io(0xdc0e, control::START | control::FORCE_LOAD);
        for _ in 0..5 {
            devices.clock_system_cycle(&mut memory).unwrap();
        }
        assert!(devices.irq_asserted());
        assert!(!devices.nmi_asserted());
        assert_eq!(
            devices.read_io(0xdc0d, 0xff),
            interrupt::SET_OR_PENDING | interrupt::TIMER_A
        );
        assert_eq!(devices.read_io(0xdd0d, 0xff), 0);
        assert_eq!(
            devices.cia1().port_a_output_pins(),
            devices.read_io(u16::from(register::PORT_A) | 0xdc00, 0xff)
        );
    }

    #[test]
    fn held_read_state_survives_hardware_cycles_until_the_next_cpu_read() {
        let mut devices = C64Chipset::new(VideoStandard::Pal);
        let mut memory = ZeroVicMemory;
        devices.begin_cpu_read();
        devices.mark_cpu_read_held();
        devices.clock_system_cycle(&mut memory).unwrap();
        assert!(devices.cpu_read_was_held());

        devices.begin_cpu_read();
        assert!(!devices.cpu_read_was_held());
    }

    #[test]
    fn cia1_port_b_fire_line_drives_the_vic_light_pen_input() {
        let mut devices = C64Chipset::new(VideoStandard::Pal);
        let mut memory = ZeroVicMemory;
        devices.write_io(0xdc03, CIA1_PORT_B_LIGHT_PEN_INPUT);
        devices.clock_system_cycle(&mut memory).unwrap();

        assert_ne!(devices.read_io(0xd019, 0xff) & 0x08, 0);
    }

    #[test]
    fn board_io_routes_all_sid_mirrors_and_clocks_the_chip() {
        let mut devices = C64Chipset::new(VideoStandard::Pal);
        let mut memory = ZeroVicMemory;
        devices.write_io(0xd500, 0x34);
        devices.write_io(0xd601, 0x12);
        devices.write_io(0xd704, sid_control::SAWTOOTH);
        devices.clock_system_cycle(&mut memory).unwrap();

        assert_eq!(
            devices
                .sid()
                .voice_state(0)
                .expect("voice exists")
                .frequency,
            0x1234
        );
        assert_eq!(devices.read_io(0xd400, 0xff), sid_control::SAWTOOTH);
        let oscillator_3 = devices.read_io(u16::from(sid_register::OSCILLATOR_3) | 0xd400, 0xff);
        assert_eq!(devices.read_io(0xd400, 0xff), oscillator_3);
    }

    #[test]
    fn cia2_uses_open_collector_iec_inputs_and_inverted_outputs() {
        let mut devices = C64Chipset::new(VideoStandard::Pal);
        let external = devices.iec_bus_mut().attach().unwrap();
        devices
            .iec_bus_mut()
            .set_port_low_mask(external, IecLine::Clock.mask() | IecLine::Data.mask())
            .unwrap();

        assert_eq!(devices.read_io(0xdd00, 0xff), 0x3f);
        devices
            .iec_bus_mut()
            .set_port_low_mask(external, 0)
            .unwrap();
        devices.write_io(0xdd02, 0x38);
        devices.write_io(0xdd00, 0x38);
        let state = devices.iec_bus().state();
        assert!(!state.attention_high());
        assert!(!state.clock_high());
        assert!(!state.data_high());

        devices.write_io(0xdd00, 0x00);
        let state = devices.iec_bus().state();
        assert!(state.attention_high());
        assert!(state.clock_high());
        assert!(state.data_high());
    }
}
