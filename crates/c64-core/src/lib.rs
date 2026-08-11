// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 确定性整机核心公共入口
//
//   文件:       lib.rs
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

#![forbid(unsafe_code)]

pub mod address_space;
pub mod architecture;
pub mod bus;
pub mod clock;
pub mod cpu;
pub mod cpu_generated;
pub mod devices;
pub mod machine;
pub mod media;
pub mod memory;
pub mod pla;
pub mod processor_port;
pub mod state;

pub use address_space::{
    C64AddressSpace, C64BusDevices, C64Firmware, CartridgeLines, CartridgeRegion,
    DisconnectedBusDevices, FirmwareError,
};
pub use architecture::{
    CoreConfig, ExecutionConfigError, ExecutionController, ExecutionRequest, ExecutionStatus,
    MachineProfile, PacingMode, SlotsPerSystemCycle, TurboSpeedRequest, VideoStandard,
};
pub use clock::{VirtualClock, VirtualClockError, VirtualTimestamp};
pub use cpu::{
    Cpu6510, Cpu6510Error, Cpu6510State, CpuBus, CpuInterruptTiming, CpuIrqLine, CpuNmiLine,
};
pub use devices::cia::{Mos6526, Mos6526Model, Mos6526Timing, Mos6526TimingError};
pub use devices::iec::{IecBus, IecBusError, IecBusState, IecBusTransition, IecLine, IecPort};
pub use devices::sid::{
    Sid, SidAudioResampler, SidConfigError, SidEnvelopeGenerator, SidExternalFilter, SidFilter,
    SidModel, SidMos6581Filter, SidMos8580Filter, SidOscillator, SidPcmRangeError,
    SidResamplerConfigError, SidTimingError, SidVoice, SidVoiceState,
};
pub use devices::via::{Mos6522, Mos6522ControlLine, Mos6522ShiftMode};
pub use devices::vic::{VicCycleResult, VicCycleSequencer, VicCycleSignals};
pub use devices::{C64Chipset, C64ChipsetError};
pub use machine::{C64Core, CoreDiagnostics, CoreError, CpuBusAccessKind, CpuBusTransaction};
pub use memory::{
    BASE_RAM_BYTES, CoherentMemory, MemoryWriteSource, PageDescriptor, PageDomain, PhysicalTarget,
};
pub use pla::{C64Pla, CartridgeMode, PlaInputs, PlaTarget};
pub use processor_port::{ProcessorPort6510, ProcessorPortInputState, ProcessorPortOutputState};
pub use state::{SAVE_STATE_FORMAT_VERSION, StateError};
