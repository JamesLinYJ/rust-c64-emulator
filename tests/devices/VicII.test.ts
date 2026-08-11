// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 寄存器与芯片状态测试
//
//   文件:       VicII.test.ts
//
//   日期:       2026年07月16日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it } from 'vitest';

import { VicII } from '../../src/devices/VicII';
import type { VicMemoryBus } from '../../src/devices/VicMemoryBus';
import { VIC_INTERRUPT_BIT, VIC_REGISTER } from '../../src/devices/vicRegisters';

const TEST_VIC_MEMORY: VicMemoryBus = {
  cpuDataBusValue: 0xff,
  readVicByte: () => 0xff,
  readVicColor: () => 0x0f,
};

describe('VicII', () => {
  it('latches a raster interrupt independently of the interrupt mask', () => {
    const vic = new VicII();
    vic.write(VIC_REGISTER.rasterCounter, 0x2a);
    while (vic.currentRasterLine !== 0x2a) vic.tickCycle(TEST_VIC_MEMORY);

    expect(vic.isRasterInterruptPending()).toBe(false);
    expect(vic.read(VIC_REGISTER.interruptStatus) & VIC_INTERRUPT_BIT.raster).toBe(
      VIC_INTERRUPT_BIT.raster,
    );

    vic.write(VIC_REGISTER.interruptMask, VIC_INTERRUPT_BIT.raster);
    expect(vic.isRasterInterruptPending()).toBe(true);
    expect(vic.read(VIC_REGISTER.interruptStatus) & VIC_INTERRUPT_BIT.any).toBe(
      VIC_INTERRUPT_BIT.any,
    );

    vic.write(VIC_REGISTER.interruptStatus, VIC_INTERRUPT_BIT.raster);
    expect(vic.isRasterInterruptPending()).toBe(false);
  });

  it('records read-to-clear sprite collision masks and their IRQ sources', () => {
    const vic = new VicII();
    vic.write(
      VIC_REGISTER.interruptMask,
      VIC_INTERRUPT_BIT.spriteSpriteCollision | VIC_INTERRUPT_BIT.spriteForegroundCollision,
    );
    vic.recordSpriteSpriteCollision(0x03);
    vic.recordSpriteForegroundCollision(0x02);

    expect(vic.isRasterInterruptPending()).toBe(true);
    expect(vic.sprites.map((sprite) => sprite.collisionWithSprite)).toEqual([
      true,
      true,
      false,
      false,
      false,
      false,
      false,
      false,
    ]);
    expect(vic.sprites.map((sprite) => sprite.collisionWithForeground)).toEqual([
      false,
      true,
      false,
      false,
      false,
      false,
      false,
      false,
    ]);
    expect(vic.read(VIC_REGISTER.spriteSpriteCollision)).toBe(0x03);
    expect(vic.read(VIC_REGISTER.spriteSpriteCollision)).toBe(0x00);
    expect(vic.read(VIC_REGISTER.spriteForegroundCollision)).toBe(0x02);
    expect(
      vic.read(VIC_REGISTER.interruptStatus) &
        (VIC_INTERRUPT_BIT.spriteSpriteCollision | VIC_INTERRUPT_BIT.spriteForegroundCollision),
    ).toBe(VIC_INTERRUPT_BIT.spriteSpriteCollision | VIC_INTERRUPT_BIT.spriteForegroundCollision);
  });

  it('only raises a collision IRQ on the register zero-to-nonzero transition', () => {
    const vic = new VicII();
    vic.write(VIC_REGISTER.interruptMask, VIC_INTERRUPT_BIT.spriteSpriteCollision);

    vic.recordSpriteSpriteCollision(0x01);
    expect(vic.interruptPending).toBe(true);
    vic.write(VIC_REGISTER.interruptStatus, VIC_INTERRUPT_BIT.spriteSpriteCollision);
    expect(vic.interruptPending).toBe(false);

    vic.recordSpriteSpriteCollision(0x02);
    expect(vic.interruptPending).toBe(false);
    expect(vic.read(VIC_REGISTER.spriteSpriteCollision)).toBe(0x03);

    vic.recordSpriteSpriteCollision(0x04);
    expect(vic.interruptPending).toBe(true);
  });

  it('latches the first light-pen position in a frame', () => {
    const vic = new VicII();
    vic.latchLightPen(100, 80);
    vic.latchLightPen(220, 120);

    expect(vic.read(VIC_REGISTER.lightPenX)).toBe(50);
    expect(vic.read(VIC_REGISTER.lightPenY)).toBe(80);
    expect(vic.read(VIC_REGISTER.interruptStatus) & VIC_INTERRUPT_BIT.lightPen).toBe(
      VIC_INTERRUPT_BIT.lightPen,
    );
  });

  it('latches the quantized PAL beam one cycle after the control-port light-pen edge', () => {
    const vic = new VicII();
    vic.setLightPenInputHigh(false);

    expect(vic.read(VIC_REGISTER.interruptStatus) & VIC_INTERRUPT_BIT.lightPen).toBe(0);
    vic.tickCycle(TEST_VIC_MEMORY);

    expect(vic.read(VIC_REGISTER.lightPenX)).toBe(0xca);
    expect(vic.read(VIC_REGISTER.lightPenY)).toBe(0x00);
    expect(vic.read(VIC_REGISTER.interruptStatus) & VIC_INTERRUPT_BIT.lightPen).toBe(
      VIC_INTERRUPT_BIT.lightPen,
    );
  });

  it('quantizes the 6569R3 light-pen X counter before applying its phase bits', () => {
    const vic = new VicII();
    for (let cycle = 0; cycle < 13; cycle += 1) vic.tickCycle(TEST_VIC_MEMORY);

    vic.setLightPenInputHigh(false);
    expect(vic.read(VIC_REGISTER.interruptStatus) & VIC_INTERRUPT_BIT.lightPen).toBe(0);
    vic.tickCycle(TEST_VIC_MEMORY);

    expect(vic.currentRasterCycle).toBe(14);
    expect(vic.read(VIC_REGISTER.lightPenX)).toBe(0x02);
    expect(vic.read(VIC_REGISTER.lightPenY)).toBe(0x00);
    expect(vic.read(VIC_REGISTER.interruptStatus) & VIC_INTERRUPT_BIT.lightPen).toBe(
      VIC_INTERRUPT_BIT.lightPen,
    );
  });

  it('returns the fixed high bits and unused-register values of a PAL VIC-II', () => {
    const vic = new VicII();
    vic.write(VIC_REGISTER.screenControl2, 0x00);
    vic.write(VIC_REGISTER.memoryPointers, 0x00);
    vic.write(VIC_REGISTER.borderColor, 0x05);
    vic.write(VIC_REGISTER.firstUnused, 0x00);

    expect(vic.read(VIC_REGISTER.screenControl2)).toBe(0xc0);
    expect(vic.read(VIC_REGISTER.memoryPointers)).toBe(0x01);
    expect(vic.read(VIC_REGISTER.borderColor)).toBe(0xf5);
    expect(vic.read(VIC_REGISTER.firstUnused)).toBe(0xff);
  });

  it('makes a border-color register write readable without advancing a dot', () => {
    const vic = new VicII();

    vic.write(VIC_REGISTER.borderColor, 0x06);

    expect(vic.currentRasterCycle).toBe(0);
    expect(vic.read(VIC_REGISTER.borderColor)).toBe(0xf6);
  });

  it('copies the complete raster line into an existing target at an explicit offset', () => {
    const vic = new VicII();
    vic.write(VIC_REGISTER.borderColor, 0x05);
    vic.tickCycle(TEST_VIC_MEMORY);
    const expected = vic.captureRasterLineState().pixels;
    const guardPixel = 0xff123456;
    const targetOffset = 2;
    const target = new Uint32Array(expected.length + 3).fill(guardPixel);

    vic.copyRasterLinePixelsTo(target, targetOffset);

    expect(target.subarray(targetOffset, targetOffset + expected.length)).toEqual(expected);
    expect(target[0]).toBe(guardPixel);
    expect(target[1]).toBe(guardPixel);
    expect(target.at(-1)).toBe(guardPixel);
    expect(() => vic.copyRasterLinePixelsTo(target, -1)).toThrow(/offset/);
    expect(() => vic.copyRasterLinePixelsTo(new Uint32Array(expected.length), 1)).toThrow(/fit/);
  });
});
