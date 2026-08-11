// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - bounded PCM playback buffer
//
//   File:       PcmAudioPlaybackBuffer.ts
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { Float32RingBuffer } from '../shared/Float32RingBuffer';
import {
  C64_PCM_BUFFER_DURATION_SECONDS,
  C64_PCM_START_BUFFER_DURATION_SECONDS,
} from './PcmAudioWorkletProtocol';

export class PcmAudioPlaybackBuffer {
  readonly startThresholdSamples: number;

  private readonly samples: Float32RingBuffer;
  private playbackStarted = false;
  private underrunSampleCount = 0;

  constructor(sampleRateHz: number) {
    if (!Number.isFinite(sampleRateHz) || sampleRateHz <= 0) {
      throw new RangeError('PCM playback sample rate must be a positive finite number.');
    }
    const capacity = Math.ceil(sampleRateHz * C64_PCM_BUFFER_DURATION_SECONDS);
    this.samples = new Float32RingBuffer(capacity);
    this.startThresholdSamples = Math.min(
      capacity,
      Math.ceil(sampleRateHz * C64_PCM_START_BUFFER_DURATION_SECONDS),
    );
  }

  get bufferedSamples(): number {
    return this.samples.size;
  }

  get capacitySamples(): number {
    return this.samples.capacity;
  }

  get underrunSamples(): number {
    return this.underrunSampleCount;
  }

  push(input: Float32Array): number {
    return this.samples.pushMany(input);
  }

  render(output: Float32Array): number {
    if (!this.playbackStarted) {
      if (this.samples.size < this.startThresholdSamples) {
        output.fill(0);
        return 0;
      }
      this.playbackStarted = true;
    }

    const written = this.samples.pullInto(output);
    if (written < output.length) {
      output.fill(0, written);
      this.underrunSampleCount += output.length - written;
    }
    return written;
  }

  clear(): void {
    this.samples.clear();
    this.playbackStarted = false;
  }
}
