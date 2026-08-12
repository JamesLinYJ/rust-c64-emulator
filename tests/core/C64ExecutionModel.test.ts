// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 公共执行模式测试
//
//   文件:       C64ExecutionModel.test.ts
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it } from 'vitest';

import {
  C64ExecutionConfigurationError,
  C64ExecutionController,
  c64EffectiveCpuClockHz,
} from '../../src/core/execution/C64ExecutionModel';

describe('C64ExecutionController', () => {
  it('starts and resets in strict mode', () => {
    const controller = new C64ExecutionController();
    controller.request({
      kind: 'turbo',
      speed: { kind: 'manual', slotsPerSystemCycle: 20 },
    });

    expect(controller.reset()).toEqual({
      requested: { kind: 'strict' },
      effectiveMode: 'strict',
      resolvedSlotsPerSystemCycle: 1,
      awaitingAutoCalibration: false,
    });
  });

  it('locks one concrete Auto tier without adapting it later', () => {
    const controller = new C64ExecutionController();
    controller.request({
      kind: 'turbo',
      speed: { kind: 'auto', maximumSlotsPerSystemCycle: 48 },
    });
    expect(controller.status.awaitingAutoCalibration).toBe(true);
    expect(controller.lockAutoSlots(24)).toMatchObject({
      effectiveMode: 'turbo',
      resolvedSlotsPerSystemCycle: 24,
      awaitingAutoCalibration: false,
    });
    expect(controller.status.resolvedSlotsPerSystemCycle).toBe(24);
  });

  it('rejects invalid and over-maximum slot values', () => {
    const controller = new C64ExecutionController();
    expect(() =>
      controller.request({
        kind: 'turbo',
        speed: { kind: 'manual', slotsPerSystemCycle: 1 },
      }),
    ).toThrow(C64ExecutionConfigurationError);

    controller.request({
      kind: 'turbo',
      speed: { kind: 'auto', maximumSlotsPerSystemCycle: 16 },
    });
    expect(() => controller.lockAutoSlots(20)).toThrow(C64ExecutionConfigurationError);
  });

  it('reports PAL and NTSC effective clocks from the locked public slot tier', () => {
    expect(c64EffectiveCpuClockHz('pal', 20)).toBe(19_704_960);
    expect(c64EffectiveCpuClockHz('ntsc', 20)).toBe(20_454_540);
  });
});
