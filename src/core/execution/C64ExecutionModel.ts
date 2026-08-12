// +-------------------------------------------------------------------------
//
//   Rust C64 模拟器 - 公共机器模式与速度请求
//
//   文件:       C64ExecutionModel.ts
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

export const C64_MACHINE_PROFILE = {
  STOCK: 'stock',
  SUPERCPU: 'supercpu',
  VM_ENHANCED: 'vm-enhanced',
} as const;

export type C64MachineProfile = (typeof C64_MACHINE_PROFILE)[keyof typeof C64_MACHINE_PROFILE];

export const C64_TIMING = {
  PAL: 'pal',
  NTSC: 'ntsc',
} as const;

export type C64Timing = (typeof C64_TIMING)[keyof typeof C64_TIMING];

export const C64_PACING = {
  REALTIME: 'realtime',
  UNBOUNDED: 'unbounded',
} as const;

export type C64Pacing = (typeof C64_PACING)[keyof typeof C64_PACING];

export const C64_TURBO_SLOT_PRESETS = Object.freeze([2, 4, 8, 12, 16, 20, 24, 32, 48, 64] as const);
export const C64_MINIMUM_TURBO_SLOTS = 2;
export const C64_MAXIMUM_TURBO_SLOTS = 64;
export const C64_STRICT_SLOTS_PER_SYSTEM_CYCLE = 1;

export interface C64StrictExecutionRequest {
  readonly kind: 'strict';
}

export interface C64ManualTurboSpeed {
  readonly kind: 'manual';
  readonly slotsPerSystemCycle: number;
}

export interface C64AutoTurboSpeed {
  readonly kind: 'auto';
  readonly maximumSlotsPerSystemCycle: number;
}

export type C64TurboSpeed = C64ManualTurboSpeed | C64AutoTurboSpeed;

export interface C64TurboExecutionRequest {
  readonly kind: 'turbo';
  readonly speed: C64TurboSpeed;
}

export type C64ExecutionRequest = C64StrictExecutionRequest | C64TurboExecutionRequest;

export interface C64ExecutionStatus {
  readonly requested: C64ExecutionRequest;
  readonly effectiveMode: 'strict' | 'turbo';
  readonly resolvedSlotsPerSystemCycle: number;
  readonly awaitingAutoCalibration: boolean;
}

export interface C64MachineConfiguration {
  readonly profile: C64MachineProfile;
  readonly timing: C64Timing;
  readonly pacing: C64Pacing;
  readonly execution: C64ExecutionRequest;
}

export const C64_DEFAULT_MACHINE_CONFIGURATION: C64MachineConfiguration = Object.freeze({
  profile: C64_MACHINE_PROFILE.STOCK,
  timing: C64_TIMING.PAL,
  pacing: C64_PACING.REALTIME,
  execution: Object.freeze({ kind: 'strict' }),
});

export class C64ExecutionConfigurationError extends RangeError {}

export class C64ExecutionController {
  private currentStatus: C64ExecutionStatus = strictStatus();

  get status(): C64ExecutionStatus {
    return this.currentStatus;
  }

  request(request: C64ExecutionRequest): C64ExecutionStatus {
    if (request.kind === 'strict') {
      this.currentStatus = strictStatus();
      return this.currentStatus;
    }

    if (request.speed.kind === 'manual') {
      const slots = validateTurboSlots(request.speed.slotsPerSystemCycle);
      this.currentStatus = Object.freeze({
        requested: freezeRequest(request),
        effectiveMode: 'turbo',
        resolvedSlotsPerSystemCycle: slots,
        awaitingAutoCalibration: false,
      });
      return this.currentStatus;
    }

    validateTurboSlots(request.speed.maximumSlotsPerSystemCycle);
    this.currentStatus = Object.freeze({
      requested: freezeRequest(request),
      effectiveMode: 'strict',
      resolvedSlotsPerSystemCycle: C64_STRICT_SLOTS_PER_SYSTEM_CYCLE,
      awaitingAutoCalibration: true,
    });
    return this.currentStatus;
  }

  lockAutoSlots(resolvedSlotsPerSystemCycle: number): C64ExecutionStatus {
    const request = this.currentStatus.requested;
    if (request.kind !== 'turbo' || request.speed.kind !== 'auto') {
      throw new C64ExecutionConfigurationError('尚未请求 Auto Turbo，不能锁定校准档位。');
    }

    const resolved =
      resolvedSlotsPerSystemCycle === C64_STRICT_SLOTS_PER_SYSTEM_CYCLE
        ? C64_STRICT_SLOTS_PER_SYSTEM_CYCLE
        : validateTurboSlots(resolvedSlotsPerSystemCycle);
    if (resolved > request.speed.maximumSlotsPerSystemCycle) {
      throw new C64ExecutionConfigurationError(
        `Auto 校准档位 ${resolved} 超过请求上限 ${request.speed.maximumSlotsPerSystemCycle}。`,
      );
    }

    this.currentStatus = Object.freeze({
      requested: request,
      effectiveMode: resolved === C64_STRICT_SLOTS_PER_SYSTEM_CYCLE ? 'strict' : 'turbo',
      resolvedSlotsPerSystemCycle: resolved,
      awaitingAutoCalibration: false,
    });
    return this.currentStatus;
  }

  reset(): C64ExecutionStatus {
    this.currentStatus = strictStatus();
    return this.currentStatus;
  }
}

export function validateTurboSlots(slotsPerSystemCycle: number): number {
  if (
    !Number.isInteger(slotsPerSystemCycle) ||
    slotsPerSystemCycle < C64_MINIMUM_TURBO_SLOTS ||
    slotsPerSystemCycle > C64_MAXIMUM_TURBO_SLOTS
  ) {
    throw new C64ExecutionConfigurationError(
      `Turbo 槽位必须是 ${C64_MINIMUM_TURBO_SLOTS}..${C64_MAXIMUM_TURBO_SLOTS} 的整数，实际为 ${slotsPerSystemCycle}。`,
    );
  }
  return slotsPerSystemCycle;
}

export function c64EffectiveCpuClockHz(
  timing: C64Timing,
  resolvedSlotsPerSystemCycle: number,
): number {
  const baseClockHz = timing === C64_TIMING.PAL ? 985_248 : 1_022_727;
  return baseClockHz * resolvedSlotsPerSystemCycle;
}

function strictStatus(): C64ExecutionStatus {
  return Object.freeze({
    requested: Object.freeze({ kind: 'strict' }),
    effectiveMode: 'strict',
    resolvedSlotsPerSystemCycle: C64_STRICT_SLOTS_PER_SYSTEM_CYCLE,
    awaitingAutoCalibration: false,
  });
}

function freezeRequest(request: C64TurboExecutionRequest): C64TurboExecutionRequest {
  return Object.freeze({
    kind: 'turbo',
    speed: Object.freeze({ ...request.speed }),
  });
}
