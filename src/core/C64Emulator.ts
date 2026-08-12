// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - 模拟器公共控制接口
//
//   文件:       C64Emulator.ts
//
//   日期:       2026年07月17日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import type { C64CartridgePort } from './memory/C64CartridgePort';
import { createCartridgeFromCrt } from '../media/CrtImageParser';
import { D64DiskImage, type D64DiskImageOptions } from '../media/D64DiskImage';
import { G64DiskImage, type G64DiskImageOptions } from '../media/G64DiskImage';
import {
  assertPrgStartCompatibility,
  installPrg,
  parsePrg,
  PRG_START_MODE,
  type InstallPrgOptions,
  type LoadedProgram,
  type PrgImage,
  type PrgStartMode,
} from '../media/PrgLoader';
import { parseTapImage, type TapImage, type TapImageParserOptions } from '../media/TapImageParser';
import { WritableTapImage, type WritableTapImageOptions } from '../media/WritableTapImage';
import {
  Commodore1541Drive,
  type Commodore1541DriveOptions,
} from '../peripherals/drive1541/Commodore1541Drive';
import type { DatasetteTapeImage } from '../peripherals/tape/Commodore1530Datasette';
import type { IecBus } from '../peripherals/iec/IecBus';
import type { SidModel } from '../devices/SidModel';
import type { C64ControlPortNumber } from '../peripherals/control/C64ControlPorts';
import {
  DEFAULT_FIRMWARE_URLS,
  fetchBinary,
  loadFirmware,
  type BinaryFetcher,
  type FirmwareUrls,
} from '../platform/FirmwareLoader';
import { TypedEventEmitter } from '../shared/TypedEventEmitter';
import { assertSha256 } from '../shared/BinaryIntegrity';
import { CanvasRenderer } from '../video/CanvasRenderer';
import { Cpu6502 } from './cpu/Cpu6502';
import type { CpuRegisters } from './cpu/CpuRegisters';
import { C64Memory, type C64CiaModels } from './memory/C64Memory';
import { WebAudioOutput, type WebAudioOutputStatus } from '../platform/WebAudioOutput';
import { BrowserC64Input } from '../platform/BrowserC64Input';
import { RealtimeEmulationLoop, type EmulationLoopState } from '../platform/RealtimeEmulationLoop';
import { hasBasicReadyPrompt } from './basicStartup';
import type { C64VideoStandard } from '../video/C64VideoStandard';

export interface C64EmulatorOptions {
  readonly cartridge?: C64CartridgePort;
  readonly canvas?: HTMLCanvasElement;
  readonly canvasHost?: HTMLElement;
  readonly ciaModels?: C64CiaModels;
  readonly debug?: boolean;
  readonly drive1541?: Omit<Commodore1541DriveOptions, 'debug' | 'iecBus'>;
  readonly sidModel?: SidModel;
  readonly fetcher?: BinaryFetcher;
  readonly firmwareUrls?: FirmwareUrls;
  readonly iecBus?: IecBus;
  readonly joystickPort?: C64ControlPortNumber | null;
  readonly keyboardTarget?: EventTarget;
  readonly audioTarget?: EventTarget;
  readonly signal?: AbortSignal;
  readonly videoStandard?: C64VideoStandard;
}

export interface C64ProgramLoadOptions extends InstallPrgOptions {
  readonly resetMachine?: boolean;
}

export interface C64RemoteProgramLoadOptions extends C64ProgramLoadOptions {
  readonly expectedSha256?: string;
  readonly signal?: AbortSignal;
}

interface EmulatorEvents {
  readonly audioState: WebAudioOutputStatus;
  readonly error: Error;
  readonly frame: { readonly frameNumber: number; readonly renderTime: number };
  readonly programLoaded: LoadedProgram;
  readonly state: EmulationLoopState;
}

const PRG_AUTOSTART_BOOT_FRAME_LIMIT = 300;

export class C64Emulator extends TypedEventEmitter<EmulatorEvents> {
  readonly cpu: Cpu6502;
  readonly drive1541: Commodore1541Drive | undefined;
  readonly input: BrowserC64Input;
  readonly memory: C64Memory;
  readonly renderer: CanvasRenderer;
  readonly runtime: RealtimeEmulationLoop;

  private readonly stopObservingUserPortDeviceSignals: () => void;
  private readonly stopObservingAudioState: () => void;
  private disposed = false;
  private resumeAfterUserPortReset = false;

  private constructor(
    memory: C64Memory,
    cpu: Cpu6502,
    renderer: CanvasRenderer,
    runtime: RealtimeEmulationLoop,
    input: BrowserC64Input,
    private readonly fetcher: BinaryFetcher,
    private readonly audioOutput: WebAudioOutput,
    drive1541: Commodore1541Drive | undefined,
  ) {
    super();
    this.memory = memory;
    this.cpu = cpu;
    this.drive1541 = drive1541;
    this.input = input;
    this.renderer = renderer;
    this.runtime = runtime;
    this.stopObservingAudioState = audioOutput.observeStatus((status) =>
      this.emit('audioState', status),
    );
    this.stopObservingUserPortDeviceSignals = memory.userPort.observeDeviceSignals(
      ({ current, previous }) => {
        if (current.resetPulledLow === previous.resetPulledLow) return;
        this.handleUserPortResetLine(current.resetPulledLow);
      },
    );
    runtime.on('frame', (event) => this.emit('frame', event));
    runtime.on('state', (state) => this.emit('state', state));
    runtime.on('error', (error) => this.emit('error', error));
    runtime.on('breakpoint', (error) => this.emit('error', error));
  }

  static async create(options: C64EmulatorOptions = {}): Promise<C64Emulator> {
    const fetcher = options.fetcher ?? fetch;
    const firmware = await loadFirmware(
      options.firmwareUrls ?? DEFAULT_FIRMWARE_URLS,
      fetcher,
      options.signal,
    );
    const memory = new C64Memory(firmware, {
      ...(options.cartridge ? { cartridge: options.cartridge } : {}),
      ...(options.ciaModels ? { ciaModels: options.ciaModels } : {}),
      debug: options.debug ?? false,
      ...(options.iecBus ? { iecBus: options.iecBus } : {}),
      ...(options.sidModel ? { sidModel: options.sidModel } : {}),
    });
    const cpu = new Cpu6502(memory);
    const drive1541 = options.drive1541
      ? new Commodore1541Drive({
          debug: options.debug ?? false,
          deviceNumber: options.drive1541.deviceNumber ?? 8,
          iecBus: memory.iecBus,
          rom: options.drive1541.rom,
        })
      : undefined;
    const canvas = options.canvas ?? document.createElement('canvas');
    if (!options.canvas && options.canvasHost) options.canvasHost.append(canvas);
    const renderer = new CanvasRenderer(canvas, memory.vic.palette[0]);
    const audioOutput = new WebAudioOutput(options.audioTarget ?? document, {
      sampleRateHz: memory.sid.sampleRateHz,
    });
    const runtime = new RealtimeEmulationLoop(cpu, memory, {
      audioSink: audioOutput,
      clockedPeripherals: drive1541 ? [drive1541.clock] : [],
      videoSink: renderer,
    });
    const input = new BrowserC64Input({
      controlPorts: memory.controlPorts,
      ...(options.joystickPort !== undefined ? { joystickPort: options.joystickPort } : {}),
      keyboard: memory.cia1.keyboard,
      restoreKeyInput: memory.restoreKey,
    });
    input.attach(options.keyboardTarget ?? document);
    return new C64Emulator(memory, cpu, renderer, runtime, input, fetcher, audioOutput, drive1541);
  }

  get state(): EmulationLoopState {
    return this.runtime.currentState;
  }

  get registers(): CpuRegisters {
    return this.cpu.getRegisters();
  }

  get basicReady(): boolean {
    return hasBasicReadyPrompt(this.memory);
  }

  get audioStatus(): WebAudioOutputStatus {
    return this.audioOutput.status;
  }

  enableAudio(): Promise<WebAudioOutputStatus> {
    return this.audioOutput.activate();
  }

  start(): void {
    if (this.memory.userPort.deviceSignals.resetPulledLow) return;
    this.runtime.start();
  }

  pause(): void {
    if (this.memory.userPort.deviceSignals.resetPulledLow) {
      this.resumeAfterUserPortReset = false;
    }
    this.runtime.pause();
  }

  toggle(): void {
    if (this.memory.userPort.deviceSignals.resetPulledLow) return;
    this.runtime.toggle();
  }

  reset(): void {
    this.runtime.resetTiming();
    this.memory.resetHardware();
    this.runtime.resetCpu();
  }

  stepInstruction(): number {
    this.requireReleasedUserPortReset();
    return this.runtime.stepInstruction();
  }

  stepFrame(): void {
    this.requireReleasedUserPortReset();
    this.runtime.stepFrame();
  }

  insertCartridge(cartridge: C64CartridgePort, resetMachine = true): void {
    this.memory.insertCartridge(cartridge);
    if (resetMachine) this.reset();
  }

  ejectCartridge(resetMachine = true): void {
    this.memory.ejectCartridge();
    if (resetMachine) this.reset();
  }

  async loadCartridge(
    url: string,
    options: { readonly resetMachine?: boolean; readonly signal?: AbortSignal } = {},
  ): Promise<C64CartridgePort> {
    const bytes = await fetchBinary(url, this.fetcher, options.signal);
    return this.loadCartridgeBytes(bytes, options.resetMachine ?? true);
  }

  loadCartridgeBytes(input: ArrayBuffer | Uint8Array, resetMachine = true): C64CartridgePort {
    const cartridge = createCartridgeFromCrt(input);
    this.insertCartridge(cartridge, resetMachine);
    return cartridge;
  }

  async loadProgram(
    url: string,
    options: C64RemoteProgramLoadOptions = {},
  ): Promise<LoadedProgram> {
    const bytes = await fetchBinary(url, this.fetcher, options.signal);
    if (options.expectedSha256 !== undefined) {
      await assertSha256(bytes, options.expectedSha256, `PRG ${url}`);
    }
    const programOptions: C64ProgramLoadOptions = {
      ...(options.entryAddress !== undefined ? { entryAddress: options.entryAddress } : {}),
      ...(options.resetMachine !== undefined ? { resetMachine: options.resetMachine } : {}),
      ...(options.startMode !== undefined ? { startMode: options.startMode } : {}),
    };
    return this.loadProgramBytesAsync(bytes, programOptions, options.signal);
  }

  loadProgramBytes(
    input: ArrayBuffer | Uint8Array,
    options: C64ProgramLoadOptions = {},
  ): LoadedProgram {
    const image = parsePrg(input);
    const context = this.prepareProgramLoad(image, options);
    if (context.resetMachine) this.bootToBasicReady();
    return this.installProgram(image, options, context);
  }

  async loadProgramBytesAsync(
    input: ArrayBuffer | Uint8Array,
    options: C64ProgramLoadOptions = {},
    signal?: AbortSignal,
  ): Promise<LoadedProgram> {
    const image = parsePrg(input);
    const context = this.prepareProgramLoad(image, options);

    try {
      if (context.resetMachine) await this.bootToBasicReadyAsync(signal);
      signal?.throwIfAborted();
      return this.installProgram(image, options, context);
    } catch (error: unknown) {
      // 取消或装载失败后保留暂停状态，避免从已复位但未完整安装的机器继续执行。
      this.runtime.pause();
      throw error;
    }
  }

  mountD64(input: ArrayBuffer | Uint8Array, options: D64DiskImageOptions = {}): D64DiskImage {
    const drive = this.drive1541;
    if (!drive) {
      throw new Error('No Commodore 1541 was configured when this emulator instance was created.');
    }
    return drive.mountD64(input, options);
  }

  ejectD64(): D64DiskImage {
    const drive = this.drive1541;
    if (!drive) {
      throw new Error('No Commodore 1541 was configured when this emulator instance was created.');
    }
    return drive.ejectD64();
  }

  mountG64(input: ArrayBuffer | Uint8Array, options: G64DiskImageOptions = {}): G64DiskImage {
    const drive = this.drive1541;
    if (!drive) {
      throw new Error('No Commodore 1541 was configured when this emulator instance was created.');
    }
    return drive.mountG64(input, options);
  }

  ejectG64(): G64DiskImage {
    const drive = this.drive1541;
    if (!drive) {
      throw new Error('No Commodore 1541 was configured when this emulator instance was created.');
    }
    return drive.ejectG64();
  }

  mountTap(input: ArrayBuffer | Uint8Array, options: TapImageParserOptions = {}): TapImage {
    const image = parseTapImage(input, options);
    this.memory.datasette.insertTape(image);
    return image;
  }

  mountBlankTap(options: WritableTapImageOptions = {}): WritableTapImage {
    const image = new WritableTapImage(options);
    this.memory.datasette.insertTape(image);
    return image;
  }

  ejectTap(): DatasetteTapeImage {
    return this.memory.datasette.ejectTape();
  }

  playTape(): void {
    this.memory.datasette.pressPlay();
  }

  recordTape(): void {
    this.memory.datasette.pressRecord();
  }

  stopTape(): void {
    this.memory.datasette.pressStop();
  }

  rewindTape(): void {
    this.memory.datasette.rewindToStart();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.runtime.dispose();
    this.stopObservingAudioState();
    this.audioOutput.dispose();
    this.input.dispose();
    this.drive1541?.dispose();
    this.stopObservingUserPortDeviceSignals();
    this.memory.dispose();
    this.clearListeners();
  }

  private handleUserPortResetLine(asserted: boolean): void {
    if (asserted) {
      this.resumeAfterUserPortReset = this.runtime.isRunning;
      this.runtime.pause();
      this.reset();
      return;
    }
    if (!this.resumeAfterUserPortReset) return;
    this.resumeAfterUserPortReset = false;
    this.runtime.start();
  }

  private requireReleasedUserPortReset(): void {
    if (this.memory.userPort.deviceSignals.resetPulledLow) {
      throw new Error('Cannot execute the C64 while the User Port RESET line is held low.');
    }
  }

  private bootToBasicReady(): void {
    let readyWasAbsent = !this.basicReady;
    for (let frame = 0; frame < PRG_AUTOSTART_BOOT_FRAME_LIMIT; frame += 1) {
      this.runtime.stepFrame();
      const ready = this.basicReady;
      if (!ready) readyWasAbsent = true;
      else if (readyWasAbsent) return;
    }
    throw new Error(
      `C64 BASIC did not reach READY within ${PRG_AUTOSTART_BOOT_FRAME_LIMIT} PAL frames.`,
    );
  }

  private async bootToBasicReadyAsync(signal?: AbortSignal): Promise<void> {
    let readyWasAbsent = !this.basicReady;
    for (let frame = 0; frame < PRG_AUTOSTART_BOOT_FRAME_LIMIT; frame += 1) {
      signal?.throwIfAborted();
      this.runtime.stepFrame();
      const ready = this.basicReady;
      if (!ready) readyWasAbsent = true;
      else if (readyWasAbsent) return;

      // 每个完整 PAL 帧后归还宿主事件循环；VIC 仍决定帧边界，硬件周期没有被跳过。
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
    throw new Error(
      `C64 BASIC did not reach READY within ${PRG_AUTOSTART_BOOT_FRAME_LIMIT} PAL frames.`,
    );
  }

  private prepareProgramLoad(
    image: PrgImage,
    options: C64ProgramLoadOptions,
  ): {
    readonly resetMachine: boolean;
    readonly resumeAfterLoad: boolean;
    readonly startMode: PrgStartMode;
  } {
    const startMode = options.startMode ?? PRG_START_MODE.basicRun;
    assertPrgStartCompatibility(image, {
      ...(options.entryAddress !== undefined ? { entryAddress: options.entryAddress } : {}),
      startMode,
    });
    const resetMachine = options.resetMachine ?? startMode !== PRG_START_MODE.none;
    const resumeAfterLoad = this.runtime.isRunning;

    if (resetMachine) {
      this.runtime.pause();
      this.reset();
    } else if (startMode === PRG_START_MODE.basicRun && !this.basicReady) {
      throw new Error('C64 BASIC must be ready before a PRG can be started with RUN.');
    }
    return { resetMachine, resumeAfterLoad, startMode };
  }

  private installProgram(
    image: PrgImage,
    options: C64ProgramLoadOptions,
    context: {
      readonly resumeAfterLoad: boolean;
      readonly startMode: PrgStartMode;
    },
  ): LoadedProgram {
    try {
      const installOptions: InstallPrgOptions =
        options.entryAddress === undefined
          ? { startMode: context.startMode }
          : { entryAddress: options.entryAddress, startMode: context.startMode };
      const loaded = installPrg(image, this.memory, this.cpu, installOptions);
      this.emit('programLoaded', loaded);
      if (context.resumeAfterLoad) this.runtime.start();
      return loaded;
    } catch (error: unknown) {
      // 装载失败后保留暂停状态，避免从已复位但未完整安装的机器继续执行。
      this.runtime.pause();
      throw error;
    }
  }
}
