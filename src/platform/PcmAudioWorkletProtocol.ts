// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - PCM AudioWorklet 消息协议
//
//   文件:       PcmAudioWorkletProtocol.ts
//
//   日期:       2026年08月10日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

export const C64_PCM_AUDIO_PROCESSOR_NAME = 'c64-pcm-stream';
export const C64_PCM_BUFFER_DURATION_SECONDS = 0.5;
export const C64_PCM_START_BUFFER_DURATION_SECONDS = 0.04;
export const C64_PCM_METRICS_INTERVAL_QUANTA = 32;

export type PcmAudioWorkletCommand =
  | { readonly type: 'clear' }
  | {
      readonly buffer: ArrayBuffer;
      readonly recycleToken: number;
      readonly sampleCount: number;
      readonly type: 'samples';
    };

export interface PcmAudioBufferRecycle {
  readonly buffer: ArrayBuffer;
  readonly recycleToken: number;
  readonly type: 'recycle';
}

export interface PcmAudioStreamMetrics {
  readonly bufferedSamples: number;
  readonly capacitySamples: number;
  readonly clearCount: number;
  readonly overrunSamples: number;
  readonly type: 'metrics';
  readonly underrunSamples: number;
}
