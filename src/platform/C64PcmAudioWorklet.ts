// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - PCM AudioWorklet 处理器
//
//   文件:       C64PcmAudioWorklet.ts
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { Float32RingBuffer } from '../shared/Float32RingBuffer';
import {
  C64_PCM_AUDIO_PROCESSOR_NAME,
  C64_PCM_BUFFER_DURATION_SECONDS,
  C64_PCM_METRICS_INTERVAL_QUANTA,
  type PcmAudioStreamMetrics,
  type PcmAudioWorkletCommand,
} from './PcmAudioWorkletProtocol';

declare const sampleRate: number;

declare abstract class AudioWorkletProcessor {
  readonly port: MessagePort;
  abstract process(
    inputs: readonly (readonly Float32Array[])[],
    outputs: readonly (readonly Float32Array[])[],
    parameters: Readonly<Record<string, Float32Array>>,
  ): boolean;
}

declare function registerProcessor(name: string, processor: new () => AudioWorkletProcessor): void;

class C64PcmAudioWorklet extends AudioWorkletProcessor {
  private readonly samples = new Float32RingBuffer(
    Math.ceil(sampleRate * C64_PCM_BUFFER_DURATION_SECONDS),
  );
  private clearCount = 0;
  private overrunSamples = 0;
  private underrunSamples = 0;
  private quantaSinceMetrics = 0;

  constructor() {
    super();
    this.port.addEventListener('message', (event: MessageEvent<PcmAudioWorkletCommand>) =>
      this.handleCommand(event.data),
    );
    this.port.start();
  }

  process(
    _inputs: readonly (readonly Float32Array[])[],
    outputs: readonly (readonly Float32Array[])[],
  ): boolean {
    const output = outputs[0]?.[0];
    if (output) {
      const written = this.samples.pullInto(output);
      if (written < output.length) {
        output.fill(0, written);
        this.underrunSamples += output.length - written;
      }
    }

    this.quantaSinceMetrics += 1;
    if (this.quantaSinceMetrics >= C64_PCM_METRICS_INTERVAL_QUANTA) this.publishMetrics();
    return true;
  }

  private handleCommand(command: PcmAudioWorkletCommand): void {
    if (command.type === 'clear') {
      this.samples.clear();
      this.clearCount += 1;
      this.publishMetrics();
      return;
    }

    const maximumSampleCount = command.buffer.byteLength / Float32Array.BYTES_PER_ELEMENT;
    if (
      !Number.isSafeInteger(command.sampleCount) ||
      command.sampleCount < 0 ||
      command.sampleCount > maximumSampleCount
    ) {
      return;
    }
    const samples = new Float32Array(command.buffer, 0, command.sampleCount);
    const dropped = this.samples.pushMany(samples);
    this.port.postMessage(
      {
        buffer: command.buffer,
        recycleToken: command.recycleToken,
        type: 'recycle',
      },
      [command.buffer],
    );
    if (dropped > 0) {
      // 延迟上限优先：满载时丢最旧样本，保留最新硬件时间线，并显式累计 overrun。
      this.overrunSamples += dropped;
      this.publishMetrics();
    }
  }

  private publishMetrics(): void {
    this.quantaSinceMetrics = 0;
    const metrics: PcmAudioStreamMetrics = {
      bufferedSamples: this.samples.size,
      capacitySamples: this.samples.capacity,
      clearCount: this.clearCount,
      overrunSamples: this.overrunSamples,
      type: 'metrics',
      underrunSamples: this.underrunSamples,
    };
    this.port.postMessage(metrics);
  }
}

registerProcessor(C64_PCM_AUDIO_PROCESSOR_NAME, C64PcmAudioWorklet);
