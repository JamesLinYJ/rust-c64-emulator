// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - 公共模块入口
//
//   文件:       index.ts
//
//   日期:       2026年07月17日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

export {
  C64Emulator,
  type C64EmulatorOptions,
  type C64ProgramLoadOptions,
  type C64RemoteProgramLoadOptions,
} from './core/C64Emulator';
export type { CpuRegisters } from './core/cpu/CpuRegisters';
export {
  C64_DEFAULT_MACHINE_CONFIGURATION,
  C64_MACHINE_PROFILE,
  C64_MAXIMUM_TURBO_SLOTS,
  C64_MINIMUM_TURBO_SLOTS,
  C64_PACING,
  C64_STRICT_SLOTS_PER_SYSTEM_CYCLE,
  C64_TIMING,
  C64_TURBO_SLOT_PRESETS,
  C64ExecutionConfigurationError,
  C64ExecutionController,
  c64EffectiveCpuClockHz,
  validateTurboSlots,
  type C64AutoTurboSpeed,
  type C64ExecutionRequest,
  type C64ExecutionStatus,
  type C64MachineConfiguration,
  type C64MachineProfile,
  type C64ManualTurboSpeed,
  type C64Pacing,
  type C64Timing,
  type C64TurboExecutionRequest,
  type C64TurboSpeed,
} from './core/execution/C64ExecutionModel';
export type { C64CartridgePort, C64CartridgeReadResult } from './core/memory/C64CartridgePort';
export { BANKED_CARTRIDGE_ROM_LAYOUT, BankedCartridgeRom } from './core/memory/BankedCartridgeRom';
export {
  Amd29F040BFlash,
  AMD_29F040B_FLASH_LAYOUT,
  AMD_29F040B_FLASH_STATE,
  type Amd29F040BFlashState,
} from './core/memory/Amd29F040BFlash';
export {
  C64_CARTRIDGE_MODE,
  C64_PLA_TARGET,
  C64Pla,
  c64CartridgeModeForLines,
  c64PlaConfigurationCode,
  type C64CartridgeMode,
  type C64PlaInputs,
  type C64PlaTarget,
} from './core/memory/C64Pla';
export type { C64CiaModels, C64Firmware, C64MemoryOptions } from './core/memory/C64Memory';
export {
  STANDARD_CARTRIDGE_ROM_LAYOUT,
  StandardRomCartridge,
  type StandardRomCartridgeOptions,
} from './core/memory/StandardRomCartridge';
export {
  EasyFlashCartridge,
  EASY_FLASH_LAYOUT,
  type EasyFlashCartridgeOptions,
} from './core/memory/EasyFlashCartridge';
export {
  MagicDeskCartridge,
  MAGIC_DESK_CARTRIDGE_BANK_COUNTS,
  type MagicDeskCartridgeOptions,
} from './core/memory/MagicDeskCartridge';
export {
  OceanCartridge,
  OCEAN_CARTRIDGE_BANK_COUNTS,
  type OceanCartridgeOptions,
} from './core/memory/OceanCartridge';
export { MOS_6526_MODEL, type Mos6526Model } from './devices/Mos6526Model';
export {
  createCartridgeFromCrt,
  createEasyFlashCartridgeFromCrt,
  createMagicDeskCartridgeFromCrt,
  createOceanCartridgeFromCrt,
  createStandardRomCartridgeFromCrt,
  CRT_CHIP_TYPE,
  CRT_HARDWARE_TYPE,
  parseCrtImage,
  type CrtChipPacket,
  type CrtHeader,
  type CrtImage,
} from './media/CrtImageParser';
export {
  D64DiskImage,
  D64_ERROR_CODE,
  D64_LAYOUT,
  d64SectorCountThroughTrack,
  d64SectorsOnTrack,
  type D64DiskId,
  type D64DiskImageOptions,
  type D64ErrorCode,
} from './media/D64DiskImage';
export {
  G64DiskImage,
  G64_LAYOUT,
  isG64SpeedZone,
  type G64ConstantSpeedMap,
  type G64DiskImageOptions,
  type G64HalfTrack,
  type G64SpeedMap,
  type G64SpeedZone,
  type G64VariableSpeedMap,
} from './media/G64DiskImage';
export { BUNDLED_PROGRAMS, type BundledProgramDescriptor } from './media/BundledProgramCatalog';
export {
  installPrg,
  parsePrg,
  PRG_START_MODE,
  type InstallPrgOptions,
  type LoadedProgram,
  type PrgImage,
  type PrgStartMode,
} from './media/PrgLoader';
export {
  parseTapImage,
  TAP_IMAGE_LAYOUT,
  TAP_VERSION,
  TAP_VIDEO_STANDARD,
  tapSourceClockHz,
  type TapImage,
  type TapImageParserOptions,
  type TapPulse,
  type TapVersion,
  type TapVideoStandard,
} from './media/TapImageParser';
export { WritableTapImage, type WritableTapImageOptions } from './media/WritableTapImage';
export {
  Commodore1541Drive,
  type Commodore1541DriveOptions,
} from './peripherals/drive1541/Commodore1541Drive';
export {
  decodeCommodoreGcr,
  D64_GCR_LAYOUT,
  d64SpeedZoneForTrack,
  encodeCommodoreGcr,
  type D64GcrSectorHeader,
  type D64GcrTrack,
} from './peripherals/drive1541/CommodoreGcr';
export {
  Commodore1530Datasette,
  DATASETTE_TRANSPORT,
  type DatasetteTapeImage,
  type DatasetteTransport,
} from './peripherals/tape/Commodore1530Datasette';
export {
  C64TapePort,
  type C64TapeDevicePort,
  type C64TapeHostSignals,
  type C64TapeReadPulse,
  type C64TapeSenseTransition,
} from './peripherals/tape/C64TapePort';
export {
  IEC_LINE,
  IecBus,
  IecBusPort,
  type IecBusObserver,
  type IecBusState,
  type IecBusTransition,
  type IecLine,
} from './peripherals/iec/IecBus';
export type { FirmwareUrls } from './platform/FirmwareLoader';
