// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 内存取数流水线
//
//   文件:       VicFetchPipeline.ts
//
//   日期:       2026年07月17日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { byte } from '../shared/numbers';
import { VIC_MATRIX_ACCESS_SOURCE, type VicMatrixAccess } from './VicBadLineController';
import type { VicCycleResult } from './VicCycleSequencer';
import type { VicMemoryBus } from './VicMemoryBus';
import { PAL_VIC_TIMING, type VicTiming } from './VicTiming';
import { VIC_SPRITE_COUNT } from './vicRegisters';

const VIC_FETCH_LAYOUT = {
  extendedBackgroundAddressMask: 0x39ff,
  extendedBackgroundIdleAddress: 0x39ff,
  idleAddress: 0x3fff,
  matrixColumnCount: 40,
  refreshPageAddress: 0x3f00,
  rowCounterMask: 0x07,
  spriteDataAddressShift: 6,
  spritePointerTableOffset: 0x03f8,
  videoCounterMask: 0x03ff,
} as const;

export interface VicFetchRegisters {
  readonly bitmapMemoryAddress: number;
  readonly bitmapMode: boolean;
  readonly characterMemoryAddress: number;
  readonly extendedBackgroundMode: boolean;
  readonly screenMemoryAddress: number;
}

export interface VicFetchSnapshot {
  readonly colorMatrix: Uint8Array;
  readonly graphics: Uint8Array;
  readonly idleState: boolean;
  readonly lastPhi1Byte: number;
  readonly lastPhi2Byte: number;
  readonly matrixIndex: number;
  readonly pendingSpriteData: Uint32Array;
  readonly pendingSpritePointers: Uint8Array;
  readonly refreshCounter: number;
  readonly rowCounter: number;
  readonly screenMatrix: Uint8Array;
  readonly spriteData: Uint32Array;
  readonly spriteDisplayMask: number;
  readonly spritePointers: Uint8Array;
  readonly videoCounter: number;
  readonly videoCounterBase: number;
}

// 取数单元消费时序器产生的半周期计划，并且只依赖 VicMemoryBus。
// 因此 VIC-II 的内部锁存行为不会反向依赖 C64Memory、Canvas 或 React。
export class VicFetchPipeline {
  private readonly colorMatrix = new Uint8Array(VIC_FETCH_LAYOUT.matrixColumnCount);
  private readonly graphics = new Uint8Array(VIC_FETCH_LAYOUT.matrixColumnCount);
  private readonly lineSpriteData = new Uint32Array(VIC_SPRITE_COUNT);
  private readonly lineSpritePointers = new Uint8Array(VIC_SPRITE_COUNT);
  private lineSpriteDisplayMask = 0;
  private readonly screenMatrix = new Uint8Array(VIC_FETCH_LAYOUT.matrixColumnCount);
  private readonly spriteData = new Uint32Array(VIC_SPRITE_COUNT);
  private readonly spritePointers = new Uint8Array(VIC_SPRITE_COUNT);
  private idleState = true;
  private lastPhi1Byte = 0xff;
  private lastPhi2Byte = 0xff;
  private matrixIndex = 0;
  private refreshCounter = 0xff;
  private rowCounter = 0;
  private videoCounter = 0;
  private videoCounterBase = 0;

  constructor(private readonly timing: VicTiming = PAL_VIC_TIMING) {
    this.reset();
  }

  // 未连接的 CPU 地址会看到 VIC-II 最近一次 φ1 取数留下的数据。
  // 该锁存值也为颜色 RAM 提供没有物理存储单元的高四位。
  get phi1DataBusValue(): number {
    return this.lastPhi1Byte;
  }

  get spriteDisplayMask(): number {
    return this.lineSpriteDisplayMask;
  }

  screenMatrixByte(column: number): number {
    return this.screenMatrix[this.requireColumn(column)]!;
  }

  colorMatrixNibble(column: number): number {
    return this.colorMatrix[this.requireColumn(column)]! & 0x0f;
  }

  graphicsByte(column: number): number {
    return this.graphics[this.requireColumn(column)]!;
  }

  spriteDataWord(spriteIndex: number): number {
    if (!Number.isInteger(spriteIndex) || spriteIndex < 0 || spriteIndex >= VIC_SPRITE_COUNT) {
      throw new RangeError(
        `VIC-II sprite index ${spriteIndex} is outside 0-${VIC_SPRITE_COUNT - 1}.`,
      );
    }
    return this.lineSpriteData[spriteIndex]!;
  }

  executeCycle(cycle: VicCycleResult, registers: VicFetchRegisters, memory: VicMemoryBus): void {
    if (cycle.frameStarted) this.beginFrame();
    if (cycle.lineStarted) this.graphics.fill(0);

    this.executePhi1(cycle, registers, memory);

    // 坏线判定发生在 φ1 取数之后、计数器更新和 φ2 矩阵取数之前。
    // 该顺序决定了动态修改 $D011 时读取的是旧行数据还是新行数据。
    if (cycle.enterDisplayState) this.idleState = false;
    if (cycle.cycle === this.timing.fetch.videoCounterReloadCycle) {
      this.videoCounter = this.videoCounterBase;
      this.matrixIndex = 0;
      if (cycle.resetRowCounter) this.rowCounter = 0;
    }
    if (cycle.lateVideoCounterReloadColumn !== undefined) {
      this.videoCounter = this.videoCounterBase;
      this.matrixIndex = cycle.lateVideoCounterReloadColumn;
    }
    if (cycle.cycle === this.timing.fetch.rowCounterUpdateCycle) {
      this.updateRowCounter(cycle.badLineCondition);
    }

    this.executePhi2(cycle, registers, memory);
    // 周期 10 的 φ2 完成后，精灵 3..7 已取完；精灵 0..2 仍保留上一行末尾的数据。
    // 此时复制得到的八个精灵数据属于同一条光栅线，供视频层统一消费。
    if (cycle.cycle === this.timing.sprite.lineDataReadyCycle) {
      this.lineSpriteData.set(this.spriteData);
      this.lineSpriteDisplayMask = cycle.spriteDisplayMask;
      this.lineSpritePointers.set(this.spritePointers);
    }
  }

  reset(): void {
    this.colorMatrix.fill(0);
    this.graphics.fill(0);
    this.lineSpriteData.fill(0);
    this.lineSpriteDisplayMask = 0;
    this.lineSpritePointers.fill(0);
    this.screenMatrix.fill(0);
    this.spriteData.fill(0);
    this.spritePointers.fill(0);
    this.idleState = true;
    this.lastPhi1Byte = 0xff;
    this.lastPhi2Byte = 0xff;
    this.matrixIndex = 0;
    this.refreshCounter = 0xff;
    this.rowCounter = 0;
    this.videoCounter = 0;
    this.videoCounterBase = 0;
  }

  snapshot(): VicFetchSnapshot {
    return {
      colorMatrix: this.colorMatrix.slice(),
      graphics: this.graphics.slice(),
      idleState: this.idleState,
      lastPhi1Byte: this.lastPhi1Byte,
      lastPhi2Byte: this.lastPhi2Byte,
      matrixIndex: this.matrixIndex,
      pendingSpriteData: this.spriteData.slice(),
      pendingSpritePointers: this.spritePointers.slice(),
      refreshCounter: this.refreshCounter,
      rowCounter: this.rowCounter,
      screenMatrix: this.screenMatrix.slice(),
      spriteData: this.lineSpriteData.slice(),
      spriteDisplayMask: this.lineSpriteDisplayMask,
      spritePointers: this.lineSpritePointers.slice(),
      videoCounter: this.videoCounter,
      videoCounterBase: this.videoCounterBase,
    };
  }

  private beginFrame(): void {
    this.refreshCounter = 0xff;
    this.videoCounter = 0;
    this.videoCounterBase = 0;
  }

  private executePhi1(
    cycle: VicCycleResult,
    registers: VicFetchRegisters,
    memory: VicMemoryBus,
  ): void {
    const fetch = cycle.busSchedule.phi1;
    switch (fetch.kind) {
      case 'refresh':
        this.lastPhi1Byte = memory.readVicByte(
          VIC_FETCH_LAYOUT.refreshPageAddress + this.refreshCounter,
        );
        this.refreshCounter = byte(this.refreshCounter - 1);
        break;
      case 'graphics':
        this.fetchGraphics(registers, memory);
        break;
      case 'idle':
        this.lastPhi1Byte = memory.readVicByte(VIC_FETCH_LAYOUT.idleAddress);
        break;
      case 'spritePointer':
        this.spritePointers[fetch.spriteIndex] = memory.readVicByte(
          registers.screenMemoryAddress +
            VIC_FETCH_LAYOUT.spritePointerTableOffset +
            fetch.spriteIndex,
        );
        this.lastPhi1Byte = this.spritePointers[fetch.spriteIndex]!;
        break;
      case 'spriteData':
        this.lastPhi1Byte = this.fetchSpriteData(
          fetch.spriteIndex,
          fetch.byteIndex,
          cycle.spriteDataOffsets.phi1,
          memory,
          this.lastPhi1Byte,
        );
        break;
    }
  }

  private executePhi2(
    cycle: VicCycleResult,
    registers: VicFetchRegisters,
    memory: VicMemoryBus,
  ): void {
    const fetch = cycle.busSchedule.phi2;
    if (cycle.matrixAccess !== undefined) {
      if (fetch?.kind !== 'matrix') {
        throw new Error(
          `VIC-II matrix access at cycle ${cycle.cycle} has no matching Phi2 bus schedule.`,
        );
      }
      this.fetchMatrix(cycle.matrixAccess, registers, memory);
      return;
    }

    if (!fetch || fetch.kind === 'matrix') return;

    this.lastPhi2Byte = this.fetchSpriteData(
      fetch.spriteIndex,
      fetch.byteIndex,
      cycle.spriteDataOffsets.phi2,
      memory,
      this.lastPhi2Byte,
    );
  }

  private fetchMatrix(
    access: VicMatrixAccess,
    registers: VicFetchRegisters,
    memory: VicMemoryBus,
  ): void {
    const index = this.requireMatrixIndex();
    if (index !== access.column) {
      throw new Error(
        `VIC-II matrix column ${access.column} does not match pipeline index ${index}.`,
      );
    }

    if (access.source === VIC_MATRIX_ACCESS_SOURCE.cpuDataBus) {
      this.lastPhi2Byte = 0xff;
      this.screenMatrix[index] = this.lastPhi2Byte;
      this.colorMatrix[index] = memory.cpuDataBusValue & 0x0f;
      return;
    }

    this.lastPhi2Byte = memory.readVicByte(registers.screenMemoryAddress + this.videoCounter);
    this.screenMatrix[index] = this.lastPhi2Byte;
    this.colorMatrix[index] = memory.readVicColor(this.videoCounter);
  }

  private fetchGraphics(registers: VicFetchRegisters, memory: VicMemoryBus): void {
    if (this.idleState) {
      const idleAddress = registers.extendedBackgroundMode
        ? VIC_FETCH_LAYOUT.extendedBackgroundIdleAddress
        : VIC_FETCH_LAYOUT.idleAddress;
      this.lastPhi1Byte = memory.readVicByte(idleAddress);
      return;
    }

    const index = this.requireMatrixIndex();
    const screenCode = this.screenMatrix[index]!;
    let address = registers.bitmapMode
      ? registers.bitmapMemoryAddress | (this.videoCounter << 3) | this.rowCounter
      : registers.characterMemoryAddress | (screenCode << 3) | this.rowCounter;
    if (registers.extendedBackgroundMode) {
      address &= VIC_FETCH_LAYOUT.extendedBackgroundAddressMask;
    }

    this.lastPhi1Byte = memory.readVicByte(address);
    this.graphics[index] = this.lastPhi1Byte;
    this.matrixIndex += 1;
    this.videoCounter = (this.videoCounter + 1) & VIC_FETCH_LAYOUT.videoCounterMask;
  }

  private fetchSpriteData(
    spriteIndex: number,
    byteIndex: 0 | 1 | 2,
    addressOffset: number | undefined,
    memory: VicMemoryBus,
    inactiveBusValue: number,
  ): number {
    const value =
      addressOffset === undefined
        ? byteIndex === 1
          ? memory.readVicByte(VIC_FETCH_LAYOUT.idleAddress)
          : inactiveBusValue
        : memory.readVicByte(
            (this.spritePointers[spriteIndex]! << VIC_FETCH_LAYOUT.spriteDataAddressShift) +
              addressOffset,
          );
    const shift = (2 - byteIndex) * 8;
    const byteMask = 0xff << shift;
    const previous = this.spriteData[spriteIndex]!;
    this.spriteData[spriteIndex] = ((previous & ~byteMask) | (value << shift)) >>> 0;
    return value;
  }

  private updateRowCounter(badLine: boolean): void {
    if (this.rowCounter === VIC_FETCH_LAYOUT.rowCounterMask) {
      this.idleState = true;
      this.videoCounterBase = this.videoCounter;
    }
    if (!this.idleState || badLine) {
      this.rowCounter = (this.rowCounter + 1) & VIC_FETCH_LAYOUT.rowCounterMask;
      this.idleState = false;
    }
  }

  private requireMatrixIndex(): number {
    if (this.matrixIndex < 0 || this.matrixIndex >= VIC_FETCH_LAYOUT.matrixColumnCount) {
      throw new RangeError(
        `VIC-II matrix index ${this.matrixIndex} is outside the 40-column fetch window.`,
      );
    }
    return this.matrixIndex;
  }

  private requireColumn(column: number): number {
    if (!Number.isInteger(column) || column < 0 || column >= VIC_FETCH_LAYOUT.matrixColumnCount) {
      throw new RangeError(
        `VIC-II display column ${column} is outside 0-${VIC_FETCH_LAYOUT.matrixColumnCount - 1}.`,
      );
    }
    return column;
  }
}
