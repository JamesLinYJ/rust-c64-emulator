// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 公开机器配置与执行模式
//
//   文件:       architecture.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

use core::fmt;

use crate::devices::sid::SidModel;

pub const MINIMUM_TURBO_SLOTS: u8 = 2;
pub const MAXIMUM_TURBO_SLOTS: u8 = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum MachineProfile {
    #[default]
    Stock = 0,
    SuperCpu = 1,
    VmEnhanced = 2,
}

impl MachineProfile {
    pub const fn code(self) -> u8 {
        match self {
            Self::Stock => 0,
            Self::SuperCpu => 1,
            Self::VmEnhanced => 2,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Stock),
            1 => Some(Self::SuperCpu),
            2 => Some(Self::VmEnhanced),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum VideoStandard {
    #[default]
    Pal = 0,
    Ntsc = 1,
}

impl VideoStandard {
    pub const PAL_SYSTEM_CLOCK_HZ: u32 = 985_248;
    pub const NTSC_SYSTEM_CLOCK_HZ: u32 = 1_022_727;

    pub const fn code(self) -> u8 {
        match self {
            Self::Pal => 0,
            Self::Ntsc => 1,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Pal),
            1 => Some(Self::Ntsc),
            _ => None,
        }
    }

    pub const fn system_clock_hz(self) -> u32 {
        match self {
            Self::Pal => Self::PAL_SYSTEM_CLOCK_HZ,
            Self::Ntsc => Self::NTSC_SYSTEM_CLOCK_HZ,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
#[repr(u8)]
pub enum PacingMode {
    #[default]
    Realtime = 0,
    Unbounded = 1,
}

impl PacingMode {
    pub const fn code(self) -> u8 {
        match self {
            Self::Realtime => 0,
            Self::Unbounded => 1,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Realtime),
            1 => Some(Self::Unbounded),
            _ => None,
        }
    }
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, wincode::SchemaRead, wincode::SchemaWrite,
)]
pub struct SlotsPerSystemCycle(u8);

impl SlotsPerSystemCycle {
    pub const STRICT: Self = Self(1);

    /// 构造一个公开 Turbo 槽位档位。
    ///
    /// # Errors
    ///
    /// 当 `value` 不在 2..=64 时返回 `TurboSlotsOutOfRange`。
    pub const fn try_turbo(value: u8) -> Result<Self, ExecutionConfigError> {
        if value < MINIMUM_TURBO_SLOTS || value > MAXIMUM_TURBO_SLOTS {
            return Err(ExecutionConfigError::TurboSlotsOutOfRange(value));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub const fn is_strict(self) -> bool {
        self.0 == Self::STRICT.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum TurboSpeedRequest {
    Manual(SlotsPerSystemCycle),
    Auto { maximum: SlotsPerSystemCycle },
}

impl TurboSpeedRequest {
    /// 构造确定的手动 Turbo 档位。
    ///
    /// # Errors
    ///
    /// 当槽位不在公开范围内时返回配置错误。
    pub const fn manual(slots: u8) -> Result<Self, ExecutionConfigError> {
        match SlotsPerSystemCycle::try_turbo(slots) {
            Ok(value) => Ok(Self::Manual(value)),
            Err(error) => Err(error),
        }
    }

    /// 构造一次性校准的 Auto Turbo 请求。
    ///
    /// # Errors
    ///
    /// 当最大槽位不在公开范围内时返回配置错误。
    pub const fn automatic(maximum: u8) -> Result<Self, ExecutionConfigError> {
        match SlotsPerSystemCycle::try_turbo(maximum) {
            Ok(value) => Ok(Self::Auto { maximum: value }),
            Err(error) => Err(error),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub enum ExecutionRequest {
    #[default]
    Strict,
    Turbo(TurboSpeedRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct CoreConfig {
    pub profile: MachineProfile,
    pub video_standard: VideoStandard,
    pub pacing: PacingMode,
    pub sid_model: SidModel,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            profile: MachineProfile::Stock,
            video_standard: VideoStandard::Pal,
            pacing: PacingMode::Realtime,
            sid_model: SidModel::Mos6581,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct ExecutionStatus {
    pub requested: ExecutionRequest,
    pub effective_slots: SlotsPerSystemCycle,
    pub awaiting_auto_calibration: bool,
}

impl ExecutionStatus {
    pub const fn is_turbo(self) -> bool {
        !self.effective_slots.is_strict()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, wincode::SchemaRead, wincode::SchemaWrite)]
pub struct ExecutionController {
    status: ExecutionStatus,
}

impl Default for ExecutionController {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionController {
    pub const fn new() -> Self {
        Self {
            status: ExecutionStatus {
                requested: ExecutionRequest::Strict,
                effective_slots: SlotsPerSystemCycle::STRICT,
                awaiting_auto_calibration: false,
            },
        }
    }

    pub const fn status(self) -> ExecutionStatus {
        self.status
    }

    pub(crate) const fn state_is_valid(self) -> bool {
        let effective = self.status.effective_slots.get();
        match self.status.requested {
            ExecutionRequest::Strict => {
                effective == SlotsPerSystemCycle::STRICT.get()
                    && !self.status.awaiting_auto_calibration
            }
            ExecutionRequest::Turbo(TurboSpeedRequest::Manual(requested)) => {
                requested.get() >= MINIMUM_TURBO_SLOTS
                    && requested.get() <= MAXIMUM_TURBO_SLOTS
                    && effective == requested.get()
                    && !self.status.awaiting_auto_calibration
            }
            ExecutionRequest::Turbo(TurboSpeedRequest::Auto { maximum }) => {
                maximum.get() >= MINIMUM_TURBO_SLOTS
                    && maximum.get() <= MAXIMUM_TURBO_SLOTS
                    && if self.status.awaiting_auto_calibration {
                        effective == SlotsPerSystemCycle::STRICT.get()
                    } else {
                        effective >= SlotsPerSystemCycle::STRICT.get() && effective <= maximum.get()
                    }
            }
        }
    }

    pub fn request(&mut self, request: ExecutionRequest) {
        self.status = match request {
            ExecutionRequest::Strict => Self::new().status,
            ExecutionRequest::Turbo(TurboSpeedRequest::Manual(slots)) => ExecutionStatus {
                requested: request,
                effective_slots: slots,
                awaiting_auto_calibration: false,
            },
            ExecutionRequest::Turbo(TurboSpeedRequest::Auto { .. }) => ExecutionStatus {
                requested: request,
                effective_slots: SlotsPerSystemCycle::STRICT,
                awaiting_auto_calibration: true,
            },
        };
    }

    /// 将 Auto 请求锁定为一个具体档位，运行中不再自适应。
    ///
    /// # Errors
    ///
    /// 未请求 Auto 或解析档位超过请求上限时返回配置错误。
    pub fn lock_auto_slots(
        &mut self,
        resolved: SlotsPerSystemCycle,
    ) -> Result<(), ExecutionConfigError> {
        let ExecutionRequest::Turbo(TurboSpeedRequest::Auto { maximum }) = self.status.requested
        else {
            return Err(ExecutionConfigError::AutoCalibrationNotRequested);
        };
        if resolved > maximum {
            return Err(ExecutionConfigError::AutoSlotsExceedMaximum {
                resolved: resolved.get(),
                maximum: maximum.get(),
            });
        }
        self.status.effective_slots = resolved;
        self.status.awaiting_auto_calibration = false;
        Ok(())
    }

    pub fn reset_to_strict(&mut self) {
        self.status = Self::new().status;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionConfigError {
    TurboSlotsOutOfRange(u8),
    AutoCalibrationNotRequested,
    AutoSlotsExceedMaximum { resolved: u8, maximum: u8 },
}

impl fmt::Display for ExecutionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TurboSlotsOutOfRange(value) => write!(
                formatter,
                "Turbo 槽位必须位于 {MINIMUM_TURBO_SLOTS}..={MAXIMUM_TURBO_SLOTS}，实际为 {value}",
            ),
            Self::AutoCalibrationNotRequested => {
                formatter.write_str("尚未请求 Auto Turbo，不能锁定校准档位")
            }
            Self::AutoSlotsExceedMaximum { resolved, maximum } => {
                write!(formatter, "Auto 校准档位 {resolved} 超过请求上限 {maximum}")
            }
        }
    }
}

impl std::error::Error for ExecutionConfigError {}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionConfigError, ExecutionController, ExecutionRequest, SlotsPerSystemCycle,
        TurboSpeedRequest,
    };

    #[test]
    fn manual_turbo_is_effective_immediately() {
        let mut controller = ExecutionController::new();
        let request = ExecutionRequest::Turbo(TurboSpeedRequest::manual(20).unwrap());
        controller.request(request);

        let status = controller.status();
        assert_eq!(status.requested, request);
        assert_eq!(status.effective_slots.get(), 20);
        assert!(status.is_turbo());
        assert!(!status.awaiting_auto_calibration);
    }

    #[test]
    fn auto_stays_strict_until_a_concrete_tier_is_locked() {
        let mut controller = ExecutionController::new();
        controller.request(ExecutionRequest::Turbo(
            TurboSpeedRequest::automatic(48).unwrap(),
        ));
        assert_eq!(
            controller.status().effective_slots,
            SlotsPerSystemCycle::STRICT
        );
        assert!(controller.status().awaiting_auto_calibration);

        controller
            .lock_auto_slots(SlotsPerSystemCycle::try_turbo(24).unwrap())
            .unwrap();
        assert_eq!(controller.status().effective_slots.get(), 24);
        assert!(!controller.status().awaiting_auto_calibration);
    }

    #[test]
    fn auto_rejects_a_tier_above_the_declared_maximum() {
        let mut controller = ExecutionController::new();
        controller.request(ExecutionRequest::Turbo(
            TurboSpeedRequest::automatic(16).unwrap(),
        ));
        let error = controller
            .lock_auto_slots(SlotsPerSystemCycle::try_turbo(20).unwrap())
            .unwrap_err();
        assert_eq!(
            error,
            ExecutionConfigError::AutoSlotsExceedMaximum {
                resolved: 20,
                maximum: 16,
            }
        );
    }

    #[test]
    fn reset_always_returns_to_strict() {
        let mut controller = ExecutionController::new();
        controller.request(ExecutionRequest::Turbo(
            TurboSpeedRequest::manual(64).unwrap(),
        ));
        controller.reset_to_strict();
        assert_eq!(controller.status(), ExecutionController::new().status());
    }
}
