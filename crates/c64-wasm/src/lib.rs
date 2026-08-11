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

use c64_core::{
    C64Core, CoreConfig, CoreError, MachineProfile, MemoryWriteSource, PacingMode, SidModel,
    VideoStandard,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct C64Vm {
    core: C64Core,
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
        })
    }

    /// 恢复 CPU Reset 状态和 Strict/1MHz。
    ///
    /// # Errors
    ///
    /// 当前不在安全提交边界时返回 JavaScript Error。
    pub fn reset(&mut self) -> Result<(), JsError> {
        self.core.reset().map_err(to_js_error)
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
        CoreError::Execution(inner) => JsError::new(&inner.to_string()),
        CoreError::Clock(inner) => JsError::new(&inner.to_string()),
        CoreError::Cpu(inner) => JsError::new(&inner.to_string()),
        CoreError::State(inner) => JsError::new(&inner.to_string()),
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
