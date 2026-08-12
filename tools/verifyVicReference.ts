// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - VIC-II 外部参考验证
//
//   文件:       verifyVicReference.ts
//
//   日期:       2026年07月17日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { PNG } from 'pngjs';

import { BreakpointError } from '../src/core/cpu/BreakpointError';
import { Cpu6502 } from '../src/core/cpu/Cpu6502';
import { C64Memory, type C64Firmware } from '../src/core/memory/C64Memory';
import { C64_PALETTE } from '../src/devices/VicII';
import { installPrg, parsePrg, PRG_START_MODE } from '../src/media/PrgLoader';
import { PalFrameScheduler } from '../src/video/PalFrameScheduler';
import { PAL_VIDEO_STANDARD } from '../src/video/palVideoStandard';
import { loadPinnedReferenceAsset } from './reference/loadPinnedReferenceAsset';

interface ReferenceAsset {
  readonly cacheFileName?: string;
  readonly fileName: string;
  readonly sha256: string;
  readonly url: string;
}

interface PixelReferenceDefinition {
  readonly description: string;
  readonly entryPoint: number;
  readonly program: ReferenceAsset;
  readonly referenceImage: ReferenceAsset;
}

interface ReferenceRunOptions {
  readonly entryPoint: number;
  readonly label: string;
  readonly onFrameStart?: () => void;
  readonly onRasterLine?: (memory: C64Memory, rasterLine: number) => void;
}

interface ReferenceRunResult {
  readonly frames: number;
  readonly totalCycles: number;
}

interface PalFrameCapture {
  readonly onFrameStart: () => void;
  readonly onRasterLine: (memory: C64Memory, rasterLine: number) => void;
  readonly pixels: Uint32Array;
}

const VICE_TEST_REVISION = 46_176;
const VICE_TEST_ROOT = `https://sourceforge.net/p/vice-emu/code/${VICE_TEST_REVISION}/tree/testprogs/VICII`;

const RASTER_IRQ_PROGRAM: ReferenceAsset = {
  fileName: 'rasterirq_hold.prg',
  sha256: '2a1d02f6a70b1a8dd17373426493d5dc29378a1442bda6df15308b8ffd5e1a94',
  url: `${VICE_TEST_ROOT}/rasterirq/rasterirq_hold.prg?format=raw`,
};

const LIGHT_PEN_TIMING_PROGRAM: ReferenceAsset = {
  cacheFileName: 'vic-light-pen-test2.prg',
  fileName: 'test2.prg',
  sha256: 'b8beff034421415f419ccf9ee640c3afbf4b5aa7d03746a95275e1401429634c',
  url: `${VICE_TEST_ROOT}/lp-trigger/test2.prg?format=raw`,
};

const LIGHT_PEN_RASTER_PROGRAM: ReferenceAsset = {
  cacheFileName: 'vic-light-pen-test1.prg',
  fileName: 'test1.prg',
  sha256: '8f41928e76d7d8e21271dbaa0fdc1dbce6c8481ed94d505c44f419aa521f3bad',
  url: `${VICE_TEST_ROOT}/lp-trigger/test1.prg?format=raw`,
};

const LIGHT_PEN_RASTER_REFERENCE_IMAGE: ReferenceAsset = {
  cacheFileName: 'vic-light-pen-test1.prg.png',
  fileName: 'test1.prg.png',
  sha256: 'c5f2d56fe7b6810c8971c0455905ccbbc9e9eb7365c7c9545ea1e64c3cde24a7',
  url: `${VICE_TEST_ROOT}/lp-trigger/references/test1.prg.png?format=raw`,
};

const DMA_DELAY_PROGRAM: ReferenceAsset = {
  fileName: 'test3-28-07.prg',
  sha256: '28297d89f31b18a432006e156df380b8677b074d5650556932d6ace2285d1847',
  url: `${VICE_TEST_ROOT}/dmadelay/test3-28-07.prg?format=raw`,
};

const DMA_DELAY_REFERENCE_IMAGE: ReferenceAsset = {
  fileName: 'test3-28-07.prg.png',
  sha256: 'de22e3d775444c915a76092237bd9a41885e1c681d3dcba39b1ac1ea7a53f655',
  url: `${VICE_TEST_ROOT}/dmadelay/references/test3-28-07.prg.png?format=raw`,
};

const SPRITE_PRIORITY_PROGRAM: ReferenceAsset = {
  cacheFileName: 'vic-sprite-priorities-test1.prg',
  fileName: 'test1.prg',
  sha256: 'a818d5f27a75bb385cef91e0b290b892dcdc98cc23fb9d8f9087fee442a68b36',
  url: `${VICE_TEST_ROOT}/spritepriorities/test1.prg?format=raw`,
};

const SPRITE_PRIORITY_REFERENCE_IMAGE: ReferenceAsset = {
  cacheFileName: 'vic-sprite-priorities-test1.prg.png',
  fileName: 'test1.prg.png',
  sha256: '6b6aa40003904789d5140c23e78f2b4c6a14a4b08c65fae6d18cc51e6f25e7b2',
  url: `${VICE_TEST_ROOT}/spritepriorities/references/test1.prg.png?format=raw`,
};

const SPRITE_DMA_54_PROGRAM: ReferenceAsset = {
  fileName: 'd017-54.prg',
  sha256: '530bbfd4398e6c2953854e2c6c3a9a209b9a3a90a2c4a8f898bfd2923f2847d7',
  url: `${VICE_TEST_ROOT}/spritedma/d017-54.prg?format=raw`,
};

const SPRITE_DMA_54_REFERENCE_IMAGE: ReferenceAsset = {
  fileName: 'd017-54.prg.png',
  sha256: '2cbdbc959ac07d7d539555318b60ea55893784898bafd0b13f5e846ee36ed9ae',
  url: `${VICE_TEST_ROOT}/spritedma/references/d017-54.prg.png?format=raw`,
};

const SPRITE_DMA_57_PROGRAM: ReferenceAsset = {
  fileName: 'd017-57.prg',
  sha256: 'a0f8773762192a690aec0c56c4946a70c5ff8c4959089cd3d25f0d025ac75a57',
  url: `${VICE_TEST_ROOT}/spritedma/d017-57.prg?format=raw`,
};

const SPRITE_DMA_57_REFERENCE_IMAGE: ReferenceAsset = {
  fileName: 'd017-57.prg.png',
  sha256: '2cbdbc959ac07d7d539555318b60ea55893784898bafd0b13f5e846ee36ed9ae',
  url: `${VICE_TEST_ROOT}/spritedma/references/d017-57.prg.png?format=raw`,
};

const PIXEL_REFERENCE_DEFINITIONS = [
  {
    description: 'light-pen synchronized border-color dot phase',
    entryPoint: 0x080d,
    program: LIGHT_PEN_RASTER_PROGRAM,
    referenceImage: LIGHT_PEN_RASTER_REFERENCE_IMAGE,
  },
  {
    description: 'dynamic bad-line DMA',
    entryPoint: 0x080d,
    program: DMA_DELAY_PROGRAM,
    referenceImage: DMA_DELAY_REFERENCE_IMAGE,
  },
  {
    description: 'hires and multicolor sprite priority',
    entryPoint: 0x080d,
    program: SPRITE_PRIORITY_PROGRAM,
    referenceImage: SPRITE_PRIORITY_REFERENCE_IMAGE,
  },
  {
    description: 'sprite vertical-expansion DMA transition at raster 54',
    entryPoint: 0x080d,
    program: SPRITE_DMA_54_PROGRAM,
    referenceImage: SPRITE_DMA_54_REFERENCE_IMAGE,
  },
  {
    description: 'sprite vertical-expansion DMA transition at raster 57',
    entryPoint: 0x080d,
    program: SPRITE_DMA_57_PROGRAM,
    referenceImage: SPRITE_DMA_57_REFERENCE_IMAGE,
  },
] as const satisfies readonly PixelReferenceDefinition[];

const RASTER_IRQ_ENTRY_POINT = 0x0815;

const VICE_TEST_EXIT_PORT = 0xd7ff;
const BOOT_FRAME_COUNT = 200;
const TEST_FRAME_LIMIT = 20;
const TRACE_CAPACITY = 24;

// VICE 的标准 PAL 截图保留 384x272 像素；本项目额外保留左侧 16 像素和底部 12 行，
// 因此比较时只裁剪输出视口，不缩放、不插值，也不忽略任何参考像素。
const VICE_PAL_REFERENCE_VIEWPORT = {
  height: 272,
  sourceX: 16,
  sourceY: 0,
  width: 384,
} as const;

// VICE 测试参考图采用 Colodore 色板。验证比较的是 16 色索引，而不是不同显示色板的 RGB，
// 从而严格保留字符、边框和逐像素时序，同时允许前端选择现代显示色彩。
const VICE_COLODORE_RGB_PALETTE = [
  0x000000, 0xffffff, 0x68372b, 0x70a4b2, 0x6f3d86, 0x588d43, 0x352879, 0xb8c76f, 0x6f4f25,
  0x433900, 0x9a6759, 0x444444, 0x6c6c6c, 0x9ad284, 0x6c5eb5, 0x959595,
] as const;

async function readBinary(path: string): Promise<Uint8Array> {
  return new Uint8Array(await readFile(resolve(path)));
}

async function loadFirmware(): Promise<C64Firmware> {
  const [basic, character, kernal] = await Promise.all([
    readBinary('public/firmware/basic.901226-01.bin'),
    readBinary('public/firmware/characters.901225-01.bin'),
    readBinary('public/firmware/kernal.901227-03.bin'),
  ]);
  return { basic, character, kernal };
}

async function loadReferenceAsset(asset: ReferenceAsset): Promise<Uint8Array> {
  return loadPinnedReferenceAsset({
    cachePath: resolve(`output/reference/${asset.cacheFileName ?? asset.fileName}`),
    name: `VICE ${asset.fileName}`,
    sha256: asset.sha256,
    url: asset.url,
  });
}

function runReferenceProgram(
  firmware: C64Firmware,
  program: Uint8Array,
  options: ReferenceRunOptions,
): ReferenceRunResult {
  const memory = new C64Memory(firmware);
  const cpu = new Cpu6502(memory);
  const scheduler = new PalFrameScheduler(cpu, memory);

  for (let frame = 0; frame < BOOT_FRAME_COUNT; frame += 1) scheduler.runFrame();
  installPrg(parsePrg(program), memory, cpu, { startMode: PRG_START_MODE.none });
  cpu.pc = options.entryPoint;

  const instructionTrace: string[] = [];
  cpu.setInstructionObserver((address, opcode) => {
    instructionTrace.push(
      `$${address.toString(16).padStart(4, '0')}:$${opcode.toString(16).padStart(2, '0')}`,
    );
    if (instructionTrace.length > TRACE_CAPACITY) instructionTrace.shift();
  });
  for (let address = 0x0000; address < 0x0200; address += 1) cpu.setBreakpoint(address);

  let exitCode: number | undefined;
  const stopObserving = memory.observeWrites(({ address, value }) => {
    if (address === VICE_TEST_EXIT_PORT) exitCode = value;
  });

  let frames = 0;
  try {
    while (exitCode === undefined && frames < TEST_FRAME_LIMIT) {
      options.onFrameStart?.();
      scheduler.runFrame((rasterLine) => options.onRasterLine?.(memory, rasterLine), true);
      frames += 1;
    }
  } catch (error: unknown) {
    if (error instanceof BreakpointError) {
      throw new Error(
        `${options.label} branched into zero page at $${error.address.toString(16).padStart(4, '0')}. Recent instructions: ${instructionTrace.join(' ')}`,
        { cause: error },
      );
    }
    throw error;
  } finally {
    stopObserving();
  }

  if (exitCode === undefined) {
    throw new Error(
      `${options.label} did not report a result within ${TEST_FRAME_LIMIT} PAL frames (PC=$${cpu.pc.toString(16).padStart(4, '0')}, raster=${memory.vic.currentRasterLine}, cycle=${memory.vic.currentRasterCycle}). Recent instructions: ${instructionTrace.join(' ')}`,
    );
  }
  if (exitCode !== 0) {
    throw new Error(
      `${options.label} failed with exit code $${exitCode.toString(16).padStart(2, '0')} after ${frames} PAL frames.`,
    );
  }

  return { frames, totalCycles: scheduler.machine.elapsedCycles };
}

function createPaletteIndex(colors: readonly number[], label: string): ReadonlyMap<number, number> {
  const indexByRgb = new Map<number, number>();
  for (let index = 0; index < colors.length; index += 1) {
    const rgb = colors[index] & 0xffffff;
    if (indexByRgb.has(rgb)) throw new Error(`${label} contains duplicate RGB value $${rgb}.`);
    indexByRgb.set(rgb, index);
  }
  return indexByRgb;
}

function compareReferenceFrame(
  label: string,
  actualPixels: Uint32Array,
  referenceBytes: Uint8Array,
): number {
  const reference = PNG.sync.read(Buffer.from(referenceBytes));
  const viewport = VICE_PAL_REFERENCE_VIEWPORT;
  if (reference.width !== viewport.width || reference.height !== viewport.height) {
    throw new Error(
      `${label} reference must be ${viewport.width}x${viewport.height}; ` +
        `received ${reference.width}x${reference.height}.`,
    );
  }

  const actualWidth = PAL_VIDEO_STANDARD.output.width;
  const actualHeight = PAL_VIDEO_STANDARD.output.height;
  if (
    viewport.sourceX + reference.width > actualWidth ||
    viewport.sourceY + reference.height > actualHeight
  ) {
    throw new Error(`${label} reference viewport is outside the PAL capture surface.`);
  }
  if (actualPixels.length !== actualWidth * actualHeight) {
    throw new Error(
      `PAL capture contains ${actualPixels.length} pixels; expected ${actualWidth * actualHeight}.`,
    );
  }

  const actualPalette = createPaletteIndex(C64_PALETTE, 'Project VIC-II palette');
  const referencePalette = createPaletteIndex(VICE_COLODORE_RGB_PALETTE, 'VICE Colodore palette');
  let mismatchCount = 0;
  let firstMismatch = '';

  for (let y = 0; y < reference.height; y += 1) {
    for (let x = 0; x < reference.width; x += 1) {
      const referenceOffset = (y * reference.width + x) * 4;
      const referenceRgb =
        (reference.data[referenceOffset] << 16) |
        (reference.data[referenceOffset + 1] << 8) |
        reference.data[referenceOffset + 2];
      const referenceColorIndex = referencePalette.get(referenceRgb);
      if (referenceColorIndex === undefined) {
        throw new Error(
          `${label} reference uses non-palette RGB ` +
            `$${referenceRgb.toString(16).padStart(6, '0')} at (${x}, ${y}).`,
        );
      }

      const actualX = x + viewport.sourceX;
      const actualY = y + viewport.sourceY;
      const actualRgb = actualPixels[actualY * actualWidth + actualX] & 0xffffff;
      const actualColorIndex = actualPalette.get(actualRgb);
      if (actualColorIndex === undefined) {
        throw new Error(
          `Project VIC-II emitted non-palette RGB $${actualRgb.toString(16).padStart(6, '0')} at (${actualX}, ${actualY}).`,
        );
      }

      if (actualColorIndex !== referenceColorIndex) {
        mismatchCount += 1;
        if (firstMismatch.length === 0) {
          firstMismatch = `first mismatch at (${x}, ${y}): expected color ${referenceColorIndex}, received ${actualColorIndex}`;
        }
      }
    }
  }

  if (mismatchCount !== 0) {
    throw new Error(
      `${label} frame differs at ${mismatchCount} of ` +
        `${reference.width * reference.height} pixels; ${firstMismatch}.`,
    );
  }
  return reference.width * reference.height;
}

function createPalFrameCapture(): PalFrameCapture {
  const { width, height, firstVisibleRaster, lastVisibleRasterExclusive } =
    PAL_VIDEO_STANDARD.output;
  const pixels = new Uint32Array(width * height);

  return {
    onFrameStart: () => pixels.fill(C64_PALETTE[0]),
    onRasterLine: (memory, rasterLine) => {
      if (rasterLine < firstVisibleRaster || rasterLine >= lastVisibleRasterExclusive) return;
      memory.vic.copyRasterLinePixelsTo(pixels, (rasterLine - firstVisibleRaster) * width);
    },
    pixels,
  };
}

async function main(): Promise<void> {
  const firmware = await loadFirmware();
  // SourceForge 会间歇性重置并发 TLS 连接；参考资产只下载一次，因此顺序获取更可复现。
  const rasterIrqProgram = await loadReferenceAsset(RASTER_IRQ_PROGRAM);
  const lightPenTimingProgram = await loadReferenceAsset(LIGHT_PEN_TIMING_PROGRAM);
  const pixelReferences = [];
  for (const definition of PIXEL_REFERENCE_DEFINITIONS) {
    pixelReferences.push({
      definition,
      program: await loadReferenceAsset(definition.program),
      referenceImage: await loadReferenceAsset(definition.referenceImage),
    });
  }

  const rasterIrq = runReferenceProgram(firmware, rasterIrqProgram, {
    entryPoint: RASTER_IRQ_ENTRY_POINT,
    label: 'VICE rasterirq_hold.prg',
  });
  console.log(
    `PASS VICE ${RASTER_IRQ_PROGRAM.fileName} (PAL raster IRQ): ${rasterIrq.frames} frames, ${rasterIrq.totalCycles.toLocaleString('en-US')} total cycles.`,
  );

  const lightPenTiming = runReferenceProgram(firmware, lightPenTimingProgram, {
    entryPoint: 0x080d,
    label: 'VICE lp-trigger/test2.prg',
  });
  console.log(
    `PASS VICE ${LIGHT_PEN_TIMING_PROGRAM.fileName} (MOS 6569 light-pen timing): ` +
      `${lightPenTiming.frames} frames, ${lightPenTiming.totalCycles.toLocaleString('en-US')} total cycles.`,
  );

  for (const { definition, program, referenceImage } of pixelReferences) {
    const capture = createPalFrameCapture();
    const label = `VICE ${definition.program.fileName}`;
    const run = runReferenceProgram(firmware, program, {
      entryPoint: definition.entryPoint,
      label,
      onFrameStart: capture.onFrameStart,
      onRasterLine: capture.onRasterLine,
    });
    const comparedPixels = compareReferenceFrame(label, capture.pixels, referenceImage);
    console.log(
      `PASS ${label} (${definition.description}): ` +
        `${comparedPixels.toLocaleString('en-US')} exact palette-index pixels, ` +
        `${run.frames} frames.`,
    );
  }

  console.log(
    `PASS VICE VIC-II pixel suite revision ${VICE_TEST_REVISION}: ` +
      `${PIXEL_REFERENCE_DEFINITIONS.length} exact PAL frame references.`,
  );
}

await main();
