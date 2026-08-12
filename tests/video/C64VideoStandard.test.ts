// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - PAL/NTSC platform timing contract
//
//   File:       C64VideoStandard.test.ts
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it } from 'vitest';

import {
  C64_VIDEO_STANDARDS,
  frameDurationMs,
  videoStandardCode,
} from '../../src/video/C64VideoStandard';

describe('C64VideoStandard', () => {
  it('locks the VICE PAL and MOS 6567R8 NTSC frame domains', () => {
    expect(C64_VIDEO_STANDARDS.pal).toMatchObject({
      code: 0,
      cyclesPerFrame: 63 * 312,
      processorClockHz: 985_248,
      rasterHeight: 284,
      rasterWidth: 403,
    });
    expect(C64_VIDEO_STANDARDS.ntsc).toMatchObject({
      code: 1,
      cyclesPerFrame: 65 * 263,
      processorClockHz: 1_022_727,
      rasterHeight: 247,
      rasterWidth: 403,
    });
    expect(videoStandardCode('pal')).toBe(0);
    expect(videoStandardCode('ntsc')).toBe(1);
  });

  it('derives host pacing from integer clock-domain metadata', () => {
    expect(frameDurationMs(985_248, 63 * 312)).toBeCloseTo(19.950_307, 6);
    expect(frameDurationMs(1_022_727, 65 * 263)).toBeCloseTo(16.715_116, 6);
    expect(() => frameDurationMs(0, 65 * 263)).toThrow(/processor clock/u);
    expect(() => frameDurationMs(1_022_727, 0)).toThrow(/frame cycle/u);
  });
});
