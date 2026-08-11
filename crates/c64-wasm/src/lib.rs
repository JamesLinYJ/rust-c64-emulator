// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - WebAssembly 粗粒度外观
//
//   文件:       lib.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

#![forbid(unsafe_code)]
// MSVC 在 native 测试链接 cdylib 时把“创建导入库”的普通状态行标为 linker_messages；
// 生产 wasm32 链接不经过该分支，核心与所有真实诊断仍由 workspace warnings=deny 管理。
#![cfg_attr(
    all(target_os = "windows", target_env = "msvc"),
    allow(linker_messages)
)]

use c64_core::devices::drive1541::mechanism::Drive1541DiskImage;
use c64_core::devices::reu::{RamExpansionUnit, ReuSize};
use c64_core::devices::sid::DEFAULT_SAMPLE_RATE_HZ;
use c64_core::devices::vic::VIC_RASTER_OUTPUT_WIDTH;
use c64_core::media::tap::TapVideoStandard;
use c64_core::{
    C64Core, C64Firmware, CoreConfig, CoreError, MachineProfile, MemoryWriteSource, PacingMode,
    SidModel, VideoStandard,
};
use wasm_bindgen::prelude::*;

const MAXIMUM_FRAME_SYSTEM_CYCLES: u64 = 25_000;
const AUDIO_BATCH_CAPACITY: usize = 2_048;
const DRIVE_DISK_FORMAT_D64: u8 = 0;
const DRIVE_DISK_FORMAT_G64: u8 = 1;

#[wasm_bindgen]
pub struct C64Vm {
    core: C64Core,
    audio_samples: Vec<f32>,
    audio_sample_count: usize,
}

#[wasm_bindgen]
impl C64Vm {
    /// 创建一个 Strict/1MHz 的独立 Wasm 整机实例。
    ///
    /// # Errors
    ///
    /// profile 或视频制式编号无效时返回 JavaScript Error。
    #[wasm_bindgen(constructor)]
    pub fn new(profile: u8, video_standard: u8) -> Result<Self, JsError> {
        let profile = MachineProfile::from_code(profile)
            .ok_or_else(|| JsError::new("无效 machine profile 编号"))?;
        let video_standard = VideoStandard::from_code(video_standard)
            .ok_or_else(|| JsError::new("无效视频制式编号"))?;
        Ok(Self {
            core: C64Core::new(CoreConfig {
                profile,
                video_standard,
                pacing: PacingMode::Realtime,
                sid_model: SidModel::Mos6581,
            }),
            audio_samples: vec![0.0; AUDIO_BATCH_CAPACITY],
            audio_sample_count: 0,
        })
    }

    /// 复制并校验真实固件后创建生产整机，固件不会跨越后续粗粒度 ABI 边界。
    ///
    /// # Errors
    ///
    /// profile、视频制式或任一 ROM 长度无效时返回 JavaScript Error。
    #[wasm_bindgen(js_name = withFirmware)]
    pub fn with_firmware(
        profile: u8,
        video_standard: u8,
        basic: &[u8],
        character: &[u8],
        kernal: &[u8],
    ) -> Result<Self, JsError> {
        let profile = MachineProfile::from_code(profile)
            .ok_or_else(|| JsError::new("无效 machine profile 编号"))?;
        let video_standard = VideoStandard::from_code(video_standard)
            .ok_or_else(|| JsError::new("无效视频制式编号"))?;
        let firmware = C64Firmware::new(basic, character, kernal)
            .map_err(|error| JsError::new(&error.to_string()))?;
        Ok(Self {
            core: C64Core::with_firmware(
                CoreConfig {
                    profile,
                    video_standard,
                    pacing: PacingMode::Realtime,
                    sid_model: SidModel::Mos6581,
                },
                firmware,
            ),
            audio_samples: vec![0.0; AUDIO_BATCH_CAPACITY],
            audio_sample_count: 0,
        })
    }

    /// 恢复 CPU Reset 状态和 Strict/1MHz。
    ///
    /// # Errors
    ///
    /// 当前不在安全提交边界时返回 JavaScript Error。
    pub fn reset(&mut self) -> Result<(), JsError> {
        self.core.reset().map_err(to_js_error)?;
        self.audio_sample_count = 0;
        Ok(())
    }

    /// 一次推进到下一完整视频帧，并把同期 SID PCM 留在复用缓冲区。
    ///
    /// # Errors
    ///
    /// 核心执行失败或在安全周期上限内没有提交帧时返回 JavaScript Error。
    pub fn run_until_next_frame(&mut self) -> Result<u32, JsError> {
        let elapsed = self
            .core
            .run_until_next_video_frame(MAXIMUM_FRAME_SYSTEM_CYCLES)
            .map_err(to_js_error)?;
        self.audio_sample_count = self.core.pull_sid_samples_into(&mut self.audio_samples);
        Ok(u32::try_from(elapsed).unwrap_or(u32::MAX))
    }

    pub fn frame_width(&self) -> u32 {
        u32::try_from(VIC_RASTER_OUTPUT_WIDTH).unwrap_or(u32::MAX)
    }

    pub fn frame_height(&self) -> u32 {
        u32::try_from(self.core.devices().vic().frame_height()).unwrap_or(u32::MAX)
    }

    pub fn video_standard(&self) -> u8 {
        self.core.config().video_standard.code()
    }

    pub fn processor_clock_hz(&self) -> u32 {
        self.core.config().video_standard.system_clock_hz()
    }

    pub fn video_frame_cycles(&self) -> u32 {
        let timing = self.core.devices().vic().timing();
        u32::from(timing.cycles_per_raster_line) * u32::from(timing.raster_line_count)
    }

    pub fn frame_pixels_ptr(&self) -> usize {
        self.core.devices().vic().frame_pixels().as_ptr() as usize
    }

    pub fn frame_pixels_len(&self) -> u32 {
        u32::try_from(self.core.devices().vic().frame_pixels().len()).unwrap_or(u32::MAX)
    }

    pub fn frame_generation_low(&self) -> u32 {
        low_u32(self.core.devices().vic().frame_generation())
    }

    pub fn audio_samples_ptr(&self) -> usize {
        self.audio_samples.as_ptr() as usize
    }

    pub fn audio_sample_count(&self) -> u32 {
        u32::try_from(self.audio_sample_count).unwrap_or(u32::MAX)
    }

    pub fn audio_sample_rate_hz(&self) -> u32 {
        DEFAULT_SAMPLE_RATE_HZ
    }

    pub fn program_counter(&self) -> u16 {
        self.core.cpu_state().program_counter
    }

    pub fn basic_ready(&self) -> bool {
        self.core.basic_ready()
    }

    /// 替换一份八列键盘矩阵、双操纵杆和 RESTORE 的宿主快照。
    ///
    /// # Errors
    ///
    /// 矩阵列数或操纵杆掩码无效时返回 JavaScript Error。
    pub fn set_host_input(
        &mut self,
        pressed_rows_by_column: &[u8],
        shift_lock_pressed: bool,
        joystick_port_1_grounded: u8,
        joystick_port_2_grounded: u8,
        restore_key_pressed: bool,
    ) -> Result<(), JsError> {
        self.core
            .set_host_input(
                pressed_rows_by_column,
                shift_lock_pressed,
                joystick_port_1_grounded,
                joystick_port_2_grounded,
                restore_key_pressed,
            )
            .map_err(to_js_error)
    }

    /// 校验并一次安装一个 `$0801` BASIC PRG，同时排入 `RUN`。
    ///
    /// # Errors
    ///
    /// 文件无效或 BASIC 尚未就绪时，在写 RAM 前返回 JavaScript Error。
    pub fn install_basic_prg(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        self.core
            .install_basic_prg(bytes)
            .map(|_| ())
            .map_err(to_js_error)
    }

    /// 在安全边界切换到 Strict。
    ///
    /// # Errors
    ///
    /// 当前不在安全提交边界时返回 JavaScript Error。
    pub fn set_strict(&mut self) -> Result<(), JsError> {
        self.core
            .request_execution(c64_core::ExecutionRequest::Strict)
            .map_err(to_js_error)
    }

    /// 请求确定的 Turbo 槽位档位。
    ///
    /// # Errors
    ///
    /// 档位超出 2..=64 或当前不在边界时返回 JavaScript Error。
    pub fn set_manual_turbo(&mut self, slots_per_system_cycle: u8) -> Result<(), JsError> {
        self.core
            .request_manual_turbo(slots_per_system_cycle)
            .map_err(to_js_error)
    }

    /// 请求一次性 Auto 校准。
    ///
    /// # Errors
    ///
    /// 上限无效或当前不在边界时返回 JavaScript Error。
    pub fn request_auto_turbo(
        &mut self,
        maximum_slots_per_system_cycle: u8,
    ) -> Result<(), JsError> {
        self.core
            .request_auto_turbo(maximum_slots_per_system_cycle)
            .map_err(to_js_error)
    }

    /// 锁定 Auto 的确定结果。
    ///
    /// # Errors
    ///
    /// 没有待校准请求、结果超限或当前不在边界时返回 JavaScript Error。
    pub fn lock_auto_turbo(&mut self, resolved_slots_per_system_cycle: u8) -> Result<(), JsError> {
        self.core
            .lock_auto_turbo(resolved_slots_per_system_cycle)
            .map_err(to_js_error)
    }

    /// 从 legacy 系统周期边界粗粒度推进整机。
    ///
    /// # Errors
    ///
    /// 当前位于周期内部时返回 JavaScript Error。
    pub fn run_system_cycles(&mut self, cycles: u32) -> Result<(), JsError> {
        self.core
            .run_system_cycles(u64::from(cycles))
            .map_err(to_js_error)
    }

    /// 推进内部 CPU 槽位。
    ///
    /// # Errors
    ///
    /// 严格 CPU 执行器检测到内部架构表不一致时返回 JavaScript Error。
    pub fn run_cpu_slots(&mut self, slots: u32) -> Result<u32, JsError> {
        let elapsed = self
            .core
            .run_cpu_slots(u64::from(slots))
            .map_err(to_js_error)?;
        Ok(u32::try_from(elapsed).unwrap_or(u32::MAX))
    }

    pub fn elapsed_system_cycles_low(&self) -> u32 {
        low_u32(self.core.timestamp().system_cycle)
    }

    pub fn elapsed_system_cycles_high(&self) -> u32 {
        high_u32(self.core.timestamp().system_cycle)
    }

    pub fn current_slot(&self) -> u8 {
        self.core.timestamp().slot
    }

    pub fn effective_slots_per_system_cycle(&self) -> u8 {
        self.core.execution_status().effective_slots.get()
    }

    pub fn awaiting_auto_calibration(&self) -> bool {
        self.core.execution_status().awaiting_auto_calibration
    }

    pub fn read_base_ram(&self, address: u16) -> u8 {
        self.core.read_base_ram(address)
    }

    pub fn write_base_ram(&mut self, address: u16, value: u8) {
        self.core
            .write_base_ram(address, value, MemoryWriteSource::HostLoader);
    }

    pub fn memory_generation_low(&self) -> u32 {
        low_u32(self.core.memory().memory_generation())
    }

    pub fn diagnostics_retired_cpu_slots_low(&self) -> u32 {
        low_u32(self.core.diagnostics().retired_cpu_slots)
    }

    pub fn diagnostics_retired_cpu_slots_high(&self) -> u32 {
        high_u32(self.core.diagnostics().retired_cpu_slots)
    }

    pub fn diagnostics_elapsed_system_cycles_low(&self) -> u32 {
        low_u32(self.core.diagnostics().elapsed_system_cycles)
    }

    pub fn diagnostics_elapsed_system_cycles_high(&self) -> u32 {
        high_u32(self.core.diagnostics().elapsed_system_cycles)
    }

    pub fn diagnostics_held_cpu_read_system_cycles_low(&self) -> u32 {
        low_u32(self.core.diagnostics().held_cpu_read_system_cycles)
    }

    pub fn diagnostics_held_cpu_read_system_cycles_high(&self) -> u32 {
        high_u32(self.core.diagnostics().held_cpu_read_system_cycles)
    }

    pub fn diagnostics_reu_dma_system_cycles_low(&self) -> u32 {
        low_u32(self.core.diagnostics().reu_dma_system_cycles)
    }

    pub fn diagnostics_reu_dma_system_cycles_high(&self) -> u32 {
        high_u32(self.core.diagnostics().reu_dma_system_cycles)
    }

    pub fn diagnostics_reu_dma_vic_stall_cycles_low(&self) -> u32 {
        low_u32(self.core.diagnostics().reu_dma_vic_stall_cycles)
    }

    pub fn diagnostics_reu_dma_vic_stall_cycles_high(&self) -> u32 {
        high_u32(self.core.diagnostics().reu_dma_vic_stall_cycles)
    }

    pub fn diagnostics_reu_dma_bus_cycles_low(&self) -> u32 {
        low_u32(self.core.diagnostics().reu_dma_bus_cycles)
    }

    pub fn diagnostics_reu_dma_bus_cycles_high(&self) -> u32 {
        high_u32(self.core.diagnostics().reu_dma_bus_cycles)
    }

    pub fn diagnostics_cpu_bus_transactions_low(&self) -> u32 {
        low_u32(self.core.diagnostics().cpu_bus_transactions)
    }

    pub fn diagnostics_cpu_bus_transactions_high(&self) -> u32 {
        high_u32(self.core.diagnostics().cpu_bus_transactions)
    }

    pub fn diagnostics_execution_mode_changes_low(&self) -> u32 {
        low_u32(self.core.diagnostics().execution_mode_changes)
    }

    pub fn diagnostics_execution_mode_changes_high(&self) -> u32 {
        high_u32(self.core.diagnostics().execution_mode_changes)
    }

    pub fn diagnostics_state_loads_low(&self) -> u32 {
        low_u32(self.core.diagnostics().state_loads)
    }

    pub fn diagnostics_state_loads_high(&self) -> u32 {
        high_u32(self.core.diagnostics().state_loads)
    }

    /// Attach one 1541 using an explicit 16 KiB DOS ROM.
    ///
    /// # Errors
    ///
    /// Rejects an invalid device number or ROM, an occupied drive slot, and
    /// IEC attachment errors.
    pub fn attach_drive1541(&mut self, device_number: u8, rom: &[u8]) -> Result<(), JsError> {
        self.core
            .attach_drive1541(device_number, rom)
            .map_err(to_js_error)
    }

    /// Detach the configured 1541.
    ///
    /// # Errors
    ///
    /// Rejects an empty drive slot or an internal CPU slot.
    pub fn detach_drive1541(&mut self) -> Result<(), JsError> {
        self.core.detach_drive1541().map_err(to_js_error)
    }

    pub fn drive1541_attached(&self) -> bool {
        self.core.drive1541_attached()
    }

    pub fn drive1541_disk_mounted(&self) -> bool {
        self.core.drive1541_disk_mounted()
    }

    pub fn drive1541_disk_write_protected(&self) -> bool {
        self.core.drive1541_disk_write_protected()
    }

    /// Parse and mount one D64 in a single coarse call.
    ///
    /// # Errors
    ///
    /// Rejects malformed media, an empty drive or an occupied mechanism.
    pub fn mount_drive1541_d64(
        &mut self,
        bytes: &[u8],
        write_protected: bool,
    ) -> Result<(), JsError> {
        self.core
            .mount_drive1541_d64(bytes, write_protected)
            .map_err(to_js_error)
    }

    /// Parse and mount one G64 in a single coarse call.
    ///
    /// # Errors
    ///
    /// Rejects malformed media, an empty drive or an occupied mechanism.
    pub fn mount_drive1541_g64(
        &mut self,
        bytes: &[u8],
        write_protected: bool,
    ) -> Result<(), JsError> {
        self.core
            .mount_drive1541_g64(bytes, write_protected)
            .map_err(to_js_error)
    }

    /// Serialize and eject one D64/G64. Byte zero identifies D64 (`0`) or
    /// G64 (`1`); all remaining bytes are the exact media image.
    ///
    /// # Errors
    ///
    /// Rejects an empty drive/mechanism, an invalid G64 export, or a D64 with
    /// uncommitted raw-track writes. Failed serialization leaves media mounted.
    pub fn eject_drive1541_disk(&mut self) -> Result<Vec<u8>, JsError> {
        let (format, bytes) = match self
            .core
            .devices()
            .drive1541()
            .and_then(|drive| drive.machine().mechanism().mounted_disk())
        {
            Some(Drive1541DiskImage::D64(image)) => (
                DRIVE_DISK_FORMAT_D64,
                image.to_bytes(image.has_error_info()),
            ),
            Some(Drive1541DiskImage::G64(image)) => (
                DRIVE_DISK_FORMAT_G64,
                image
                    .to_bytes()
                    .map_err(|error| JsError::new(&error.to_string()))?,
            ),
            None => return Err(JsError::new("no 1541 disk is mounted")),
        };
        self.core.eject_drive1541_disk().map_err(to_js_error)?;
        let mut result = Vec::with_capacity(bytes.len() + 1);
        result.push(format);
        result.extend_from_slice(&bytes);
        Ok(result)
    }

    /// Parse and attach one CRT image in a single coarse ABI call.
    ///
    /// # Errors
    ///
    /// Rejects malformed media, unsupported hardware or an occupied slot.
    pub fn insert_crt(
        &mut self,
        bytes: &[u8],
        easy_flash_jumper_installed: bool,
    ) -> Result<(), JsError> {
        self.core
            .insert_crt(bytes, easy_flash_jumper_installed)
            .map_err(to_js_error)
    }

    /// Detach the expansion-port cartridge.
    ///
    /// # Errors
    ///
    /// Rejects an empty slot or an internal CPU slot.
    pub fn eject_cartridge(&mut self) -> Result<(), JsError> {
        self.core.eject_cartridge().map(|_| ()).map_err(to_js_error)
    }

    pub fn cartridge_attached(&self) -> bool {
        self.core.cartridge_attached()
    }

    pub fn cartridge_kind(&self) -> u16 {
        self.core
            .cartridge_kind()
            .map_or(u16::MAX, c64_core::devices::cartridge::CartridgeKind::code)
    }

    pub fn easyflash_dirty(&self) -> bool {
        self.core.easyflash_dirty().unwrap_or(false)
    }

    /// Copy the physical `EasyFlash` ROML chip in one coarse persistence call.
    ///
    /// # Errors
    ///
    /// Requires an attached `EasyFlash` board.
    pub fn export_easyflash_low(&self) -> Result<Vec<u8>, JsError> {
        self.core.export_easyflash_low().map_err(to_js_error)
    }

    /// Copy the physical `EasyFlash` ROMH chip in one coarse persistence call.
    ///
    /// # Errors
    ///
    /// Requires an attached `EasyFlash` board.
    pub fn export_easyflash_high(&self) -> Result<Vec<u8>, JsError> {
        self.core.export_easyflash_high().map_err(to_js_error)
    }

    /// Attach an empty classic 128, 256 or 512 KiB 17xx REU.
    ///
    /// # Errors
    ///
    /// Rejects unsupported sizes, an occupied expansion port or an internal
    /// CPU slot.
    pub fn attach_reu(&mut self, size_kib: u16) -> Result<(), JsError> {
        let size = ReuSize::from_kibibytes(size_kib)
            .ok_or_else(|| JsError::new("REU size must be 128, 256 or 512 KiB"))?;
        self.core.attach_reu(size).map_err(to_js_error)
    }

    /// Attach a classic REU and initialize all of its physical DRAM in one call.
    ///
    /// # Errors
    ///
    /// Rejects unsupported sizes, a mismatched image, an occupied expansion
    /// port or an internal CPU slot.
    pub fn attach_reu_image(&mut self, size_kib: u16, image: &[u8]) -> Result<(), JsError> {
        let size = ReuSize::from_kibibytes(size_kib)
            .ok_or_else(|| JsError::new("REU size must be 128, 256 or 512 KiB"))?;
        self.core.attach_reu_image(size, image).map_err(to_js_error)
    }

    /// Detach the REU and return its complete physical DRAM image.
    ///
    /// # Errors
    ///
    /// Requires an attached REU and a system-cycle boundary.
    pub fn detach_reu(&mut self) -> Result<Vec<u8>, JsError> {
        self.core
            .detach_reu()
            .map(|reu| reu.ram().to_vec())
            .map_err(to_js_error)
    }

    pub fn reu_attached(&self) -> bool {
        self.core.reu().is_some()
    }

    pub fn reu_size_kib(&self) -> u16 {
        self.core.reu().map_or(0, |reu| reu.size().kibibytes())
    }

    pub fn reu_dma_active(&self) -> bool {
        self.core.reu().is_some_and(RamExpansionUnit::dma_active)
    }

    /// Copy the complete physical REU DRAM in one coarse persistence call.
    ///
    /// # Errors
    ///
    /// Requires an attached REU.
    pub fn export_reu_ram(&self) -> Result<Vec<u8>, JsError> {
        self.core.export_reu_ram().map_err(to_js_error)
    }

    /// Parse and insert a read-only TAP image. A zero legacy duration means
    /// that ambiguous TAP v0 zero markers remain rejected.
    ///
    /// # Errors
    ///
    /// Rejects malformed media, duplicate insertion or a moving transport.
    pub fn insert_tap(
        &mut self,
        bytes: &[u8],
        legacy_v0_overflow_pulse_cycles: u32,
    ) -> Result<(), JsError> {
        let legacy_duration =
            (legacy_v0_overflow_pulse_cycles != 0).then_some(legacy_v0_overflow_pulse_cycles);
        self.core
            .insert_tap(bytes, legacy_duration)
            .map_err(to_js_error)
    }

    /// Insert an empty writable TAP v1 image.
    ///
    /// # Errors
    ///
    /// Rejects an invalid TAP video-standard code or duplicate media.
    pub fn insert_blank_tap(&mut self, video_standard: u8) -> Result<(), JsError> {
        let video_standard = TapVideoStandard::try_from(video_standard)
            .map_err(|error| JsError::new(&error.to_string()))?;
        self.core
            .insert_blank_tap(video_standard)
            .map_err(to_js_error)
    }

    /// Serialize and eject stopped media in one coarse ABI call.
    ///
    /// # Errors
    ///
    /// Requires stopped mounted media.
    pub fn eject_tap(&mut self) -> Result<Vec<u8>, JsError> {
        self.core.eject_tap().map_err(to_js_error)
    }

    /// Engage the physical PLAY key.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot.
    pub fn tape_play(&mut self) -> Result<(), JsError> {
        self.core.tape_play().map_err(to_js_error)
    }

    /// Engage the physical RECORD key.
    ///
    /// # Errors
    ///
    /// Requires stopped writable media.
    pub fn tape_record(&mut self) -> Result<(), JsError> {
        self.core.tape_record().map_err(to_js_error)
    }

    /// Stop the physical transport.
    ///
    /// # Errors
    ///
    /// Rejects an internal CPU slot.
    pub fn tape_stop(&mut self) -> Result<(), JsError> {
        self.core.tape_stop().map_err(to_js_error)
    }

    /// Rewind mounted media.
    ///
    /// # Errors
    ///
    /// Requires stopped mounted media.
    pub fn tape_rewind(&mut self) -> Result<(), JsError> {
        self.core.tape_rewind().map_err(to_js_error)
    }

    /// Seek to a physical TAP pulse boundary.
    ///
    /// # Errors
    ///
    /// Requires stopped mounted media and an in-range index.
    pub fn tape_seek_pulse(&mut self, pulse_index: u32) -> Result<(), JsError> {
        self.core
            .tape_seek_pulse(
                usize::try_from(pulse_index)
                    .map_err(|_| JsError::new("TAP pulse index exceeds the host address range"))?,
            )
            .map_err(to_js_error)
    }

    pub fn tape_mounted(&self) -> bool {
        self.core.tape_mounted()
    }

    pub fn tape_writable(&self) -> bool {
        self.core.tape_writable()
    }

    pub fn tape_pulse_count(&self) -> u32 {
        u32::try_from(self.core.tape_pulse_count()).unwrap_or(u32::MAX)
    }

    pub fn tape_pulse_index(&self) -> u32 {
        u32::try_from(self.core.tape_pulse_index()).unwrap_or(u32::MAX)
    }

    pub fn tape_transport(&self) -> u8 {
        self.core.tape_transport().code()
    }

    pub fn tape_motor_active(&self) -> bool {
        self.core.tape_motor_active()
    }

    pub fn tape_sense_switch_closed(&self) -> bool {
        self.core.tape_sense_switch_closed()
    }

    pub fn save_state(&self) -> Vec<u8> {
        self.core.save_state()
    }

    /// 载入经过版本和边界验证的架构状态。
    ///
    /// # Errors
    ///
    /// 状态格式、长度、版本或内部不变量无效时返回 JavaScript Error。
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        self.core.load_state(bytes).map_err(to_js_error)
    }
}

fn to_js_error(error: CoreError) -> JsError {
    match error {
        CoreError::Chipset(inner) => JsError::new(&inner.to_string()),
        CoreError::Execution(inner) => JsError::new(&inner.to_string()),
        CoreError::Clock(inner) => JsError::new(&inner.to_string()),
        CoreError::Cpu(inner) => JsError::new(&inner.to_string()),
        CoreError::State(inner) => JsError::new(&inner.to_string()),
        CoreError::Prg(inner) => JsError::new(&inner.to_string()),
        timeout @ CoreError::VideoFrameTimeout { .. } => JsError::new(&timeout.to_string()),
        CoreError::Vic(inner) => JsError::new(&inner.to_string()),
    }
}

fn low_u32(value: u64) -> u32 {
    let bytes = value.to_le_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn high_u32(value: u64) -> u32 {
    let bytes = value.to_le_bytes();
    u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])
}
