// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - PCM playback priming root-cause tests
//
//   File:       PcmAudioPlaybackBuffer.test.ts
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

import { describe, expect, it } from 'vitest';

import { PcmAudioPlaybackBuffer } from '../../src/platform/PcmAudioPlaybackBuffer';

describe('PcmAudioPlaybackBuffer', () => {
  it('does not classify startup silence as a stream underrun', () => {
    const playback = new PcmAudioPlaybackBuffer(44_100);
    const output = new Float32Array(128).fill(1);

    expect(playback.render(output)).toBe(0);
    expect(output).toEqual(new Float32Array(128));
    expect(playback.underrunSamples).toBe(0);

    playback.push(new Float32Array(playback.startThresholdSamples - 1).fill(0.25));
    expect(playback.render(output)).toBe(0);
    expect(playback.underrunSamples).toBe(0);
  });

  it('starts after the bounded prebuffer and counts only a later real starvation', () => {
    const playback = new PcmAudioPlaybackBuffer(44_100);
    const input = new Float32Array(playback.startThresholdSamples).fill(0.5);
    const firstOutput = new Float32Array(128);
    expect(playback.push(input)).toBe(0);
    expect(playback.render(firstOutput)).toBe(128);
    expect(firstOutput.every((sample) => sample === 0.5)).toBe(true);
    expect(playback.underrunSamples).toBe(0);

    playback.render(new Float32Array(playback.startThresholdSamples));
    expect(playback.underrunSamples).toBeGreaterThan(0);

    playback.clear();
    playback.render(new Float32Array(128));
    expect(playback.underrunSamples).toBeGreaterThan(0);
  });
});
