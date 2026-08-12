// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - platform video-standard metadata
//
//   File:       C64VideoStandard.ts
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

export type C64VideoStandard = 'ntsc' | 'pal';

export interface C64VideoStandardDefinition {
  readonly code: 0 | 1;
  readonly cyclesPerFrame: number;
  readonly label: 'NTSC' | 'PAL';
  readonly processorClockHz: number;
  readonly rasterHeight: number;
  readonly rasterWidth: number;
  readonly refreshRateHz: number;
}

const PAL_PROCESSOR_CLOCK_HZ = 985_248;
const PAL_CYCLES_PER_FRAME = 63 * 312;
const NTSC_PROCESSOR_CLOCK_HZ = 1_022_727;
const NTSC_CYCLES_PER_FRAME = 65 * 263;

export const C64_VIDEO_STANDARDS = {
  ntsc: {
    code: 1,
    cyclesPerFrame: NTSC_CYCLES_PER_FRAME,
    label: 'NTSC',
    processorClockHz: NTSC_PROCESSOR_CLOCK_HZ,
    rasterHeight: 247,
    rasterWidth: 403,
    refreshRateHz: NTSC_PROCESSOR_CLOCK_HZ / NTSC_CYCLES_PER_FRAME,
  },
  pal: {
    code: 0,
    cyclesPerFrame: PAL_CYCLES_PER_FRAME,
    label: 'PAL',
    processorClockHz: PAL_PROCESSOR_CLOCK_HZ,
    rasterHeight: 284,
    rasterWidth: 403,
    refreshRateHz: PAL_PROCESSOR_CLOCK_HZ / PAL_CYCLES_PER_FRAME,
  },
} as const satisfies Readonly<Record<C64VideoStandard, C64VideoStandardDefinition>>;

export function videoStandardCode(videoStandard: C64VideoStandard): 0 | 1 {
  return C64_VIDEO_STANDARDS[videoStandard].code;
}

export function frameDurationMs(processorClockHz: number, frameCycles: number): number {
  if (!Number.isSafeInteger(processorClockHz) || processorClockHz <= 0) {
    throw new RangeError('C64 processor clock must be a positive integer.');
  }
  if (!Number.isSafeInteger(frameCycles) || frameCycles <= 0) {
    throw new RangeError('C64 video frame cycle count must be a positive integer.');
  }
  return (frameCycles * 1000) / processorClockHz;
}
