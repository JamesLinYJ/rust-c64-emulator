// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - SID reSID 外部参考验证
//
//   文件:       verifySidReference.ts
//
//   日期:       2026年07月16日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { basename, resolve } from 'node:path';

import { SidEnvelopeGenerator } from '../src/devices/SidEnvelopeGenerator';
import { SidExternalFilter } from '../src/devices/SidExternalFilter';
import { SidFilter } from '../src/devices/SidFilter';
import { SID_MODEL, type SidModel } from '../src/devices/SidModel';
import { SidOscillator } from '../src/devices/SidOscillator';
import { SID_CONTROL_BIT, SID_FILTER_BIT } from '../src/devices/sidRegisters';
import { runBoundedCommand } from './runRustJsonTrace';

const RESID_REVISION = 'dd98b495dd4b49612922fdba20ad71304361cd1f';
const RESID_RAW_ROOT = `https://raw.githubusercontent.com/VICE-Team/svn-mirror/${RESID_REVISION}/vice/src/resid`;
const REFERENCE_SOURCE_DIRECTORY = resolve('output/reference/resid-source');
const REFERENCE_BUILD_DIRECTORY = resolve('output/reference/resid-build');
const LOCAL_VICE_MIRROR = resolve('output/reference/vice-source');
const ORACLE_CONFIGURATION_DIRECTORY = resolve('tools/reference/resid');
const ENVELOPE_ORACLE_SOURCE = resolve('tools/reference/resid/ResIdEnvelopeOracle.cpp');
const ENVELOPE_ORACLE_EXECUTABLE = resolve(
  REFERENCE_BUILD_DIRECTORY,
  process.platform === 'win32' ? 'ResIdEnvelopeOracle.exe' : 'ResIdEnvelopeOracle',
);
const OSCILLATOR_ORACLE_SOURCE = resolve('tools/reference/resid/ResIdOscillatorOracle.cpp');
const OSCILLATOR_ORACLE_EXECUTABLE = resolve(
  REFERENCE_BUILD_DIRECTORY,
  process.platform === 'win32' ? 'ResIdOscillatorOracle.exe' : 'ResIdOscillatorOracle',
);
const FILTER_ORACLE_SOURCE = resolve('tools/reference/resid/ResIdFilterOracle.cpp');
const FILTER_ORACLE_EXECUTABLE = resolve(
  REFERENCE_BUILD_DIRECTORY,
  process.platform === 'win32' ? 'ResIdFilterOracle.exe' : 'ResIdFilterOracle',
);
const EXTERNAL_FILTER_ORACLE_SOURCE = resolve(
  'tools/reference/resid/ResIdExternalFilterOracle.cpp',
);
const EXTERNAL_FILTER_ORACLE_EXECUTABLE = resolve(
  REFERENCE_BUILD_DIRECTORY,
  process.platform === 'win32' ? 'ResIdExternalFilterOracle.exe' : 'ResIdExternalFilterOracle',
);

interface ReferenceSource {
  readonly fileName: string;
  readonly sha256: string;
}

const REFERENCE_SOURCES: readonly ReferenceSource[] = [
  {
    fileName: 'envelope.h',
    sha256: '52fdf21883cd05110e78a7fbb98f215cb950c2eb83574153e38bf0d9f1e7c0da',
  },
  {
    fileName: 'envelope.cc',
    sha256: '35a0c17737ec37aeb40a929ddac28a4f20885e73e4478e8aacd15f50ba172dc9',
  },
  {
    fileName: 'dac.h',
    sha256: '9fa86d5c2fd3e5644871de1cdfc0eb7296b7604f68845bd7861dc16ba8d21e6b',
  },
  {
    fileName: 'dac.cc',
    sha256: '1b1133be332283dad798234fad913094e3ac222bb49f5be6a14d58e9842034fb',
  },
  {
    fileName: 'resid-config.h',
    sha256: 'ccd0047dbaf8265e7a58b2fb4232dacfa441ffa7072f42eff80552f52feafe52',
  },
  {
    fileName: 'wave.h',
    sha256: '8b916531e274c97d5eb651ed546c02ec5f8ac7a6a024d6a4db97602dd147cf9f',
  },
  {
    fileName: 'wave.cc',
    sha256: 'fbbc5c590ecf343ff70a54ce7e4861a0f674a05fe27561c497c61ad749f63673',
  },
  {
    fileName: 'filter.h',
    sha256: '61eb34bb7f104cc4a14e73eb940a0e5737b94e0c126075320702e426624b89ef',
  },
  {
    fileName: 'filter.cc',
    sha256: 'ab6d96909324ca93f9606d830d7d363750f0c3af31211c34d64d30b776277ffc',
  },
  {
    fileName: 'extfilt.h',
    sha256: 'c4b6a8d8269c54ed3e8e487d28f80270ab5120b98e9346c3d0c4caeea28cd99b',
  },
  {
    fileName: 'extfilt.cc',
    sha256: 'ce4ea29de782fcd29ad6862c1b5b72f981f2f9a0afbc1f3c94133040298b896d',
  },
  {
    fileName: 'spline.h',
    sha256: '60005cd321c0acf006548e682a4b0f4e3736070b1fac6a47c579fe1f6b8d1dea',
  },
  {
    fileName: 'wave6581__ST.dat',
    sha256: 'bd63e518ecd09b836911e2d4609256afeb28603c3a2a57c1314d0b5a928343ad',
  },
  {
    fileName: 'wave6581_P_T.dat',
    sha256: '47ac5fd90e701cdd3fa1bd7f7db90f8ce8ef98169cd17d8c5f8a26545c3944eb',
  },
  {
    fileName: 'wave6581_PS_.dat',
    sha256: '466043fe9fcfed21f81fa632e8a12d7ab9d4a971a78ba7a19d501102cf658e04',
  },
  {
    fileName: 'wave6581_PST.dat',
    sha256: '9fe871586d092ff8fd4daa88c9e784d0451f6d6832b96e5697f18ee671d102d6',
  },
  {
    fileName: 'wave8580__ST.dat',
    sha256: '3049a3197ee321b962282e01e905c3ee6b0432db346ef106f41d7c9406e181c7',
  },
  {
    fileName: 'wave8580_P_T.dat',
    sha256: 'ffa33e7959bf724a7dced993e282501b70ef35a8f50fd7e4f7b5c0eaad106941',
  },
  {
    fileName: 'wave8580_PS_.dat',
    sha256: '6db028bb1bd523bdfdd70a045722e8545618c9471de28fd685cda001cfc442b1',
  },
  {
    fileName: 'wave8580_PST.dat',
    sha256: 'ec324d749e726a19e8cc924e06fe2b4005bc02eaf42efb89323c5a25945027cf',
  },
] as const;

type EnvelopeCommand =
  | { readonly kind: 'attackDecay'; readonly value: number }
  | { readonly kind: 'clock'; readonly cycles: number }
  | { readonly kind: 'control'; readonly value: number }
  | { readonly kind: 'reset' }
  | { readonly kind: 'sustainRelease'; readonly value: number };

interface EnvelopeScenario {
  readonly commands: readonly EnvelopeCommand[];
  readonly name: string;
}

const ENVELOPE_SCENARIOS: readonly EnvelopeScenario[] = [
  {
    name: 'attack and decay pipeline',
    commands: [
      { kind: 'attackDecay', value: 0x00 },
      { kind: 'sustainRelease', value: 0x80 },
      { kind: 'control', value: 0x01 },
      { kind: 'clock', cycles: 3_000 },
    ],
  },
  {
    name: 'release exponential divider',
    commands: [
      { kind: 'attackDecay', value: 0x00 },
      { kind: 'sustainRelease', value: 0x00 },
      { kind: 'control', value: 0x01 },
      { kind: 'clock', cycles: 1_000 },
      { kind: 'control', value: 0x00 },
      { kind: 'clock', cycles: 6_000 },
    ],
  },
  {
    name: 'ADSR delay bug after a faster rate write',
    commands: [
      { kind: 'attackDecay', value: 0x90 },
      { kind: 'sustainRelease', value: 0xf0 },
      { kind: 'control', value: 0x01 },
      { kind: 'clock', cycles: 800 },
      { kind: 'attackDecay', value: 0x00 },
      { kind: 'clock', cycles: 33_000 },
    ],
  },
  {
    name: 'gate transitions and RES counter retention',
    commands: [
      { kind: 'attackDecay', value: 0x21 },
      { kind: 'sustainRelease', value: 0x34 },
      { kind: 'control', value: 0x01 },
      { kind: 'clock', cycles: 600 },
      { kind: 'control', value: 0x00 },
      { kind: 'clock', cycles: 17 },
      { kind: 'control', value: 0x01 },
      { kind: 'clock', cycles: 600 },
      { kind: 'reset' },
      { kind: 'clock', cycles: 300 },
    ],
  },
] as const;

type OscillatorCommand =
  | { readonly kind: 'clock'; readonly cycles: number }
  | { readonly index: 0 | 1 | 2; readonly kind: 'control'; readonly value: number }
  | { readonly index: 0 | 1 | 2; readonly kind: 'frequency'; readonly value: number }
  | { readonly index: 0 | 1 | 2; readonly kind: 'pulseWidth'; readonly value: number }
  | { readonly kind: 'reset' };

interface OscillatorScenario {
  readonly commands: readonly OscillatorCommand[];
  readonly model: SidModel;
  readonly name: string;
}

const OSCILLATOR_SCENARIOS: readonly OscillatorScenario[] = [
  {
    name: 'MOS 6581 sawtooth phase progression',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'frequency', index: 2, value: 0x1234 },
      { kind: 'control', index: 2, value: SID_CONTROL_BIT.sawtooth },
      { kind: 'clock', cycles: 5_000 },
    ],
  },
  {
    name: 'MOS 8580 triangle readback pipeline',
    model: SID_MODEL.mos8580,
    commands: [
      { kind: 'frequency', index: 2, value: 0x4321 },
      { kind: 'control', index: 2, value: SID_CONTROL_BIT.triangle },
      { kind: 'clock', cycles: 5_000 },
    ],
  },
  {
    name: 'pulse comparator pipeline and live width writes',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'frequency', index: 1, value: 0x7fff },
      { kind: 'pulseWidth', index: 1, value: 0x0800 },
      { kind: 'control', index: 1, value: SID_CONTROL_BIT.pulse },
      { kind: 'clock', cycles: 3_000 },
      { kind: 'pulseWidth', index: 1, value: 0x0200 },
      { kind: 'clock', cycles: 3_000 },
    ],
  },
  {
    name: 'noise shift-register two-cycle pipeline',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'frequency', index: 0, value: 0xffff },
      { kind: 'control', index: 0, value: SID_CONTROL_BIT.noise },
      { kind: 'clock', cycles: 20_000 },
    ],
  },
  {
    name: 'three-oscillator hard sync and ring modulation',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'frequency', index: 0, value: 0x7100 },
      { kind: 'frequency', index: 1, value: 0x9300 },
      { kind: 'frequency', index: 2, value: 0xb500 },
      {
        kind: 'control',
        index: 0,
        value:
          SID_CONTROL_BIT.triangle | SID_CONTROL_BIT.ringModulation | SID_CONTROL_BIT.synchronize,
      },
      {
        kind: 'control',
        index: 1,
        value: SID_CONTROL_BIT.sawtooth | SID_CONTROL_BIT.synchronize,
      },
      { kind: 'control', index: 2, value: SID_CONTROL_BIT.triangle },
      { kind: 'clock', cycles: 20_000 },
    ],
  },
  {
    name: 'MOS 6581 measured combined waveforms',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'frequency', index: 0, value: 0x3579 },
      { kind: 'frequency', index: 1, value: 0x2468 },
      { kind: 'frequency', index: 2, value: 0x789a },
      { kind: 'pulseWidth', index: 1, value: 0x0600 },
      {
        kind: 'control',
        index: 0,
        value: SID_CONTROL_BIT.triangle | SID_CONTROL_BIT.sawtooth,
      },
      {
        kind: 'control',
        index: 1,
        value: SID_CONTROL_BIT.pulse | SID_CONTROL_BIT.sawtooth,
      },
      {
        kind: 'control',
        index: 2,
        value: SID_CONTROL_BIT.noise | SID_CONTROL_BIT.pulse,
      },
      { kind: 'clock', cycles: 12_000 },
    ],
  },
  {
    name: 'MOS 8580 measured combined waveforms',
    model: SID_MODEL.mos8580,
    commands: [
      { kind: 'frequency', index: 0, value: 0x3579 },
      { kind: 'frequency', index: 1, value: 0x2468 },
      { kind: 'frequency', index: 2, value: 0x789a },
      { kind: 'pulseWidth', index: 1, value: 0x0600 },
      {
        kind: 'control',
        index: 0,
        value: SID_CONTROL_BIT.triangle | SID_CONTROL_BIT.sawtooth,
      },
      {
        kind: 'control',
        index: 1,
        value: SID_CONTROL_BIT.pulse | SID_CONTROL_BIT.sawtooth,
      },
      {
        kind: 'control',
        index: 2,
        value: SID_CONTROL_BIT.noise | SID_CONTROL_BIT.pulse,
      },
      { kind: 'clock', cycles: 12_000 },
    ],
  },
] as const;

type FilterCommand =
  | { readonly cutoff: number; readonly kind: 'cutoff' }
  | { readonly cycles: number; readonly kind: 'clock' }
  | { readonly kind: 'modeVolume'; readonly value: number }
  | { readonly kind: 'reset' }
  | { readonly kind: 'resonanceRouting'; readonly value: number }
  | { readonly kind: 'voices'; readonly values: readonly [number, number, number] };

interface FilterScenario {
  readonly commands: readonly FilterCommand[];
  readonly model: SidModel;
  readonly name: string;
}

const FILTER_SCENARIOS: readonly FilterScenario[] = [
  {
    name: 'MOS 8580 direct mixer and live voice levels',
    model: SID_MODEL.mos8580,
    commands: [
      { kind: 'reset' },
      { kind: 'modeVolume', value: 0x0f },
      { kind: 'voices', values: [100_000, -50_000, 25_000] },
      { kind: 'clock', cycles: 64 },
      { kind: 'voices', values: [-220_000, 180_000, 80_000] },
      { kind: 'clock', cycles: 64 },
    ],
  },
  {
    name: 'MOS 8580 low-pass impulse and resonance',
    model: SID_MODEL.mos8580,
    commands: [
      { kind: 'reset' },
      { kind: 'cutoff', cutoff: 0x0640 },
      { kind: 'resonanceRouting', value: 0xa0 | SID_FILTER_BIT.voice1 },
      { kind: 'modeVolume', value: SID_FILTER_BIT.lowPass | 0x0f },
      { kind: 'voices', values: [400_000, 0, 0] },
      { kind: 'clock', cycles: 1 },
      { kind: 'voices', values: [0, 0, 0] },
      { kind: 'clock', cycles: 2_047 },
    ],
  },
  {
    name: 'MOS 8580 combined modes and cutoff write',
    model: SID_MODEL.mos8580,
    commands: [
      { kind: 'reset' },
      { kind: 'cutoff', cutoff: 0x0180 },
      { kind: 'resonanceRouting', value: 0xe0 | SID_FILTER_BIT.voice2 },
      {
        kind: 'modeVolume',
        value: SID_FILTER_BIT.lowPass | SID_FILTER_BIT.bandPass | SID_FILTER_BIT.highPass | 0x0c,
      },
      { kind: 'voices', values: [90_000, 330_000, -120_000] },
      { kind: 'clock', cycles: 1_024 },
      { kind: 'cutoff', cutoff: 0x0700 },
      { kind: 'clock', cycles: 1_024 },
    ],
  },
  {
    name: 'MOS 8580 voice-three-off routing rule',
    model: SID_MODEL.mos8580,
    commands: [
      { kind: 'reset' },
      { kind: 'cutoff', cutoff: 0x07ff },
      { kind: 'resonanceRouting', value: 0x70 | SID_FILTER_BIT.voice3 },
      {
        kind: 'modeVolume',
        value: SID_FILTER_BIT.muteVoice3 | SID_FILTER_BIT.highPass | 0x0f,
      },
      { kind: 'voices', values: [0, 0, 450_000] },
      { kind: 'clock', cycles: 512 },
    ],
  },
  {
    name: 'MOS 6581 nonlinear direct mixer and live voice levels',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'reset' },
      { kind: 'modeVolume', value: 0x0f },
      { kind: 'voices', values: [100_000, -50_000, 25_000] },
      { kind: 'clock', cycles: 64 },
      { kind: 'voices', values: [-220_000, 180_000, 80_000] },
      { kind: 'clock', cycles: 64 },
    ],
  },
  {
    name: 'MOS 6581 nonlinear low-pass impulse and resonance',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'reset' },
      { kind: 'cutoff', cutoff: 0x0640 },
      { kind: 'resonanceRouting', value: 0xa0 | SID_FILTER_BIT.voice1 },
      { kind: 'modeVolume', value: SID_FILTER_BIT.lowPass | 0x0f },
      { kind: 'voices', values: [400_000, 0, 0] },
      { kind: 'clock', cycles: 1 },
      { kind: 'voices', values: [0, 0, 0] },
      { kind: 'clock', cycles: 2_047 },
    ],
  },
  {
    name: 'MOS 6581 nonlinear combined modes and cutoff write',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'reset' },
      { kind: 'cutoff', cutoff: 0x0180 },
      { kind: 'resonanceRouting', value: 0xe0 | SID_FILTER_BIT.voice2 },
      {
        kind: 'modeVolume',
        value: SID_FILTER_BIT.lowPass | SID_FILTER_BIT.bandPass | SID_FILTER_BIT.highPass | 0x0c,
      },
      { kind: 'voices', values: [90_000, 330_000, -120_000] },
      { kind: 'clock', cycles: 1_024 },
      { kind: 'cutoff', cutoff: 0x0700 },
      { kind: 'clock', cycles: 1_024 },
    ],
  },
  {
    name: 'MOS 6581 nonlinear voice-three-off routing rule',
    model: SID_MODEL.mos6581,
    commands: [
      { kind: 'reset' },
      { kind: 'cutoff', cutoff: 0x07ff },
      { kind: 'resonanceRouting', value: 0x70 | SID_FILTER_BIT.voice3 },
      {
        kind: 'modeVolume',
        value: SID_FILTER_BIT.muteVoice3 | SID_FILTER_BIT.highPass | 0x0f,
      },
      { kind: 'voices', values: [0, 0, 450_000] },
      { kind: 'clock', cycles: 512 },
    ],
  },
] as const;

type ExternalFilterCommand =
  | { readonly cycles: number; readonly kind: 'clock' }
  | { readonly kind: 'input'; readonly value: number }
  | { readonly kind: 'reset' };

interface ExternalFilterScenario {
  readonly commands: readonly ExternalFilterCommand[];
  readonly name: string;
}

const EXTERNAL_FILTER_SCENARIOS: readonly ExternalFilterScenario[] = [
  {
    name: 'positive impulse response',
    commands: [
      { kind: 'reset' },
      { kind: 'input', value: 0x4000 },
      { kind: 'clock', cycles: 2 },
      { kind: 'input', value: 0 },
      { kind: 'clock', cycles: 2_046 },
    ],
  },
  {
    name: 'signed full-scale transitions',
    commands: [
      { kind: 'reset' },
      { kind: 'input', value: -0x8000 },
      { kind: 'clock', cycles: 1_024 },
      { kind: 'input', value: 0x7fff },
      { kind: 'clock', cycles: 1_024 },
      { kind: 'input', value: -1 },
      { kind: 'clock', cycles: 257 },
    ],
  },
  {
    name: 'high-pass DC settling',
    commands: [
      { kind: 'reset' },
      { kind: 'input', value: 12_000 },
      { kind: 'clock', cycles: 32_768 },
    ],
  },
] as const;

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

function isMissingFile(error: unknown): boolean {
  return (
    error instanceof Error &&
    'code' in error &&
    (error as Error & { readonly code?: unknown }).code === 'ENOENT'
  );
}

async function loadReferenceSource(source: ReferenceSource): Promise<void> {
  const cachePath = resolve(REFERENCE_SOURCE_DIRECTORY, source.fileName);
  try {
    const cached = new Uint8Array(await readFile(cachePath));
    const actualHash = sha256(cached);
    if (actualHash !== source.sha256) {
      throw new Error(
        `Cached reSID source ${source.fileName} failed SHA-256 validation: ${actualHash}.`,
      );
    }
    return;
  } catch (error: unknown) {
    if (!isMissingFile(error)) throw error;
  }

  // 已有 VICE 镜像时直接读取固定提交的 Git 对象，避免工作树换行规则改变参考源码字节。
  const localSource = readLocalViceSource(source);
  if (localSource !== undefined) {
    await writeFile(cachePath, localSource);
    return;
  }

  let response: Response;
  try {
    response = await fetch(`${RESID_RAW_ROOT}/${source.fileName}`);
  } catch (error: unknown) {
    throw new Error(`Unable to download reSID ${source.fileName}.`, { cause: error });
  }
  if (!response.ok) {
    throw new Error(`Unable to download reSID ${source.fileName}: HTTP ${response.status}.`);
  }
  const downloaded = new Uint8Array(await response.arrayBuffer());
  const actualHash = sha256(downloaded);
  if (actualHash !== source.sha256) {
    throw new Error(`reSID ${source.fileName} SHA-256 mismatch: received ${actualHash}.`);
  }
  await writeFile(cachePath, downloaded);
}

function readLocalViceSource(source: ReferenceSource): Uint8Array | undefined {
  const revision = spawnSync('git', ['-C', LOCAL_VICE_MIRROR, 'rev-parse', 'HEAD'], {
    encoding: 'utf8',
  });
  if (revision.error || revision.status !== 0) return undefined;

  const actualRevision = revision.stdout.trim();
  if (actualRevision !== RESID_REVISION) {
    throw new Error(
      `Local VICE mirror revision ${actualRevision} does not match required ${RESID_REVISION}.`,
    );
  }

  const object = spawnSync(
    'git',
    ['-C', LOCAL_VICE_MIRROR, 'show', `${RESID_REVISION}:vice/src/resid/${source.fileName}`],
    { maxBuffer: 2 * 1024 * 1024 },
  );
  if (object.error || object.status !== 0) {
    throw new Error(
      `Unable to read ${source.fileName} from the fixed local VICE mirror: ${String(object.stderr)}`,
      object.error ? { cause: object.error } : undefined,
    );
  }

  const bytes = new Uint8Array(object.stdout);
  const actualHash = sha256(bytes);
  if (actualHash !== source.sha256) {
    throw new Error(
      `Local VICE object ${source.fileName} failed SHA-256 validation: ${actualHash}.`,
    );
  }
  return bytes;
}

const WAVEFORM_DATA_FILES = [
  'wave6581__ST.dat',
  'wave6581_P_T.dat',
  'wave6581_PS_.dat',
  'wave6581_PST.dat',
  'wave8580__ST.dat',
  'wave8580_P_T.dat',
  'wave8580_PS_.dat',
  'wave8580_PST.dat',
] as const;

async function buildOracles(): Promise<void> {
  await mkdir(REFERENCE_SOURCE_DIRECTORY, { recursive: true });
  await mkdir(REFERENCE_BUILD_DIRECTORY, { recursive: true });
  await Promise.all(REFERENCE_SOURCES.map(loadReferenceSource));
  await generateWaveformHeaders();

  await compileOracle(ENVELOPE_ORACLE_SOURCE, ENVELOPE_ORACLE_EXECUTABLE, [
    'envelope.cc',
    'dac.cc',
  ]);
  await compileOracle(OSCILLATOR_ORACLE_SOURCE, OSCILLATOR_ORACLE_EXECUTABLE, [
    'wave.cc',
    'dac.cc',
  ]);
  await compileOracle(FILTER_ORACLE_SOURCE, FILTER_ORACLE_EXECUTABLE, ['filter.cc', 'dac.cc']);
  await compileOracle(EXTERNAL_FILTER_ORACLE_SOURCE, EXTERNAL_FILTER_ORACLE_EXECUTABLE, [
    'extfilt.cc',
  ]);
}

async function generateWaveformHeaders(): Promise<void> {
  for (const dataFile of WAVEFORM_DATA_FILES) {
    const samples = new Uint8Array(await readFile(resolve(REFERENCE_SOURCE_DIRECTORY, dataFile)));
    const expectedSampleCount = 1 << 12;
    if (samples.length !== expectedSampleCount) {
      throw new RangeError(
        `reSID waveform source ${dataFile} has ${samples.length} samples; expected ${expectedSampleCount}.`,
      );
    }

    const lines = ['// 由已校验的 reSID OSC3 八位采样生成；数值左移四位恢复十二位表。', '{'];
    for (let offset = 0; offset < samples.length; offset += 8) {
      const values: string[] = [];
      for (let index = offset; index < Math.min(offset + 8, samples.length); index += 1) {
        const sample = samples[index];
        if (sample === undefined) {
          throw new Error(`Missing reSID waveform sample ${index} in ${dataFile}.`);
        }
        values.push(`0x${(sample << 4).toString(16).padStart(3, '0')},`);
      }
      lines.push(`  ${values.join(' ')}`);
    }
    lines.push('},', '');
    await writeFile(
      resolve(REFERENCE_SOURCE_DIRECTORY, dataFile.replace(/\.dat$/u, '.h')),
      lines.join('\n'),
      'utf8',
    );
  }
}

async function compileOracle(
  source: string,
  executable: string,
  referenceImplementations: readonly string[],
): Promise<void> {
  const compilerArguments = [
    '-std=c++17',
    '-O2',
    '-I',
    ORACLE_CONFIGURATION_DIRECTORY,
    '-I',
    REFERENCE_SOURCE_DIRECTORY,
    source,
    ...referenceImplementations.map((fileName) => resolve(REFERENCE_SOURCE_DIRECTORY, fileName)),
    '-o',
    executable,
  ];
  await runBoundedCommand({
    arguments: compilerArguments,
    command: 'g++',
    label: `Build reSID oracle ${basename(source)}`,
    maximumOutputBytes: 16 * 1024 * 1024,
  });
}

function serializeCommands(commands: readonly EnvelopeCommand[]): string {
  return commands
    .map((command) => {
      switch (command.kind) {
        case 'attackDecay':
          return `ATTACK_DECAY ${command.value}`;
        case 'clock':
          return `CLOCK ${command.cycles}`;
        case 'control':
          return `CONTROL ${command.value}`;
        case 'reset':
          return 'RESET';
        case 'sustainRelease':
          return `SUSTAIN_RELEASE ${command.value}`;
      }
    })
    .join('\n');
}

async function runEnvelopeOracle(commands: readonly EnvelopeCommand[]): Promise<number[]> {
  const stdout = await runBoundedCommand({
    arguments: [],
    command: ENVELOPE_ORACLE_EXECUTABLE,
    input: serializeCommands(commands),
    label: `reSID oracle ${basename(ENVELOPE_ORACLE_EXECUTABLE)}`,
    maximumOutputBytes: 16 * 1024 * 1024,
  });
  const output = stdout.trim();
  return output.length === 0 ? [] : output.split(/\s+/u).map(Number);
}

function runTypeScript(commands: readonly EnvelopeCommand[]): number[] {
  const envelope = new SidEnvelopeGenerator();
  const output: number[] = [];
  for (const command of commands) {
    switch (command.kind) {
      case 'attackDecay':
        envelope.writeAttackDecay(command.value);
        break;
      case 'clock':
        for (let cycle = 0; cycle < command.cycles; cycle += 1) {
          envelope.clock();
          output.push(envelope.readback);
        }
        break;
      case 'control':
        envelope.writeControl(command.value);
        break;
      case 'reset':
        envelope.reset();
        break;
      case 'sustainRelease':
        envelope.writeSustainRelease(command.value);
        break;
    }
  }
  return output;
}

async function verifyScenario(scenario: EnvelopeScenario): Promise<number> {
  const reference = await runEnvelopeOracle(scenario.commands);
  const actual = runTypeScript(scenario.commands);
  if (actual.length !== reference.length) {
    throw new Error(
      `${scenario.name}: TypeScript produced ${actual.length} cycles, reSID produced ${reference.length}.`,
    );
  }
  for (let cycle = 0; cycle < reference.length; cycle += 1) {
    if (actual[cycle] !== reference[cycle]) {
      throw new Error(
        `${scenario.name}: envelope mismatch at sampled cycle ${cycle + 1}; TypeScript=${String(actual[cycle])}, reSID=${String(reference[cycle])}.`,
      );
    }
  }
  return actual.length;
}

function serializeOscillatorScenario(scenario: OscillatorScenario): string {
  const commands = scenario.commands.map((command) => {
    switch (command.kind) {
      case 'clock':
        return `CLOCK ${command.cycles}`;
      case 'control':
        return `CONTROL ${command.index} ${command.value}`;
      case 'frequency':
        return `FREQUENCY ${command.index} ${command.value}`;
      case 'pulseWidth':
        return `PULSE_WIDTH ${command.index} ${command.value}`;
      case 'reset':
        return 'RESET';
    }
  });
  return [`MODEL ${scenario.model}`, ...commands].join('\n');
}

async function runOscillatorOracle(scenario: OscillatorScenario): Promise<number[]> {
  const stdout = await runBoundedCommand({
    arguments: [],
    command: OSCILLATOR_ORACLE_EXECUTABLE,
    input: serializeOscillatorScenario(scenario),
    label: `reSID oracle ${basename(OSCILLATOR_ORACLE_EXECUTABLE)}`,
    maximumOutputBytes: 16 * 1024 * 1024,
  });
  const output = stdout.trim();
  return output.length === 0 ? [] : output.split(/\s+/u).map(Number);
}

function runTypeScriptOscillators(scenario: OscillatorScenario): number[] {
  const oscillators: [SidOscillator, SidOscillator, SidOscillator] = [
    new SidOscillator(scenario.model),
    new SidOscillator(scenario.model),
    new SidOscillator(scenario.model),
  ];
  oscillators[0].setSyncSource(oscillators[2]);
  oscillators[1].setSyncSource(oscillators[0]);
  oscillators[2].setSyncSource(oscillators[1]);

  const output: number[] = [];
  for (const command of scenario.commands) {
    switch (command.kind) {
      case 'clock':
        for (let cycle = 0; cycle < command.cycles; cycle += 1) {
          for (const oscillator of oscillators) oscillator.clock();
          for (const oscillator of oscillators) oscillator.synchronizeDestination();
          for (const oscillator of oscillators) oscillator.updateWaveformOutput();
          for (const oscillator of oscillators) output.push(oscillator.oscillatorReadback);
        }
        break;
      case 'control':
        oscillators[command.index].setControl(command.value);
        break;
      case 'frequency':
        oscillators[command.index].frequency = command.value;
        break;
      case 'pulseWidth':
        oscillators[command.index].pulseWidth = command.value;
        break;
      case 'reset':
        for (const oscillator of oscillators) oscillator.reset();
        break;
    }
  }
  return output;
}

async function verifyOscillatorScenario(scenario: OscillatorScenario): Promise<number> {
  const reference = await runOscillatorOracle(scenario);
  const actual = runTypeScriptOscillators(scenario);
  if (actual.length !== reference.length) {
    throw new Error(
      `${scenario.name}: TypeScript produced ${actual.length} oscillator samples, reSID produced ${reference.length}.`,
    );
  }
  for (let sample = 0; sample < reference.length; sample += 1) {
    if (actual[sample] !== reference[sample]) {
      const voice = sample % 3;
      const cycle = Math.trunc(sample / 3) + 1;
      throw new Error(
        `${scenario.name}: oscillator ${voice} mismatch at sampled cycle ${cycle}; TypeScript=${String(actual[sample])}, reSID=${String(reference[sample])}.`,
      );
    }
  }
  return actual.length;
}

function serializeFilterScenarios(scenarios: readonly FilterScenario[]): string {
  return scenarios
    .flatMap((scenario) => [
      `MODEL ${scenario.model}`,
      ...scenario.commands.map((command) => {
        switch (command.kind) {
          case 'clock':
            return `CLOCK ${command.cycles}`;
          case 'cutoff':
            return `CUTOFF ${command.cutoff}`;
          case 'modeVolume':
            return `MODE_VOLUME ${command.value}`;
          case 'reset':
            return 'RESET';
          case 'resonanceRouting':
            return `RESONANCE_ROUTING ${command.value}`;
          case 'voices':
            return `VOICES ${command.values.join(' ')}`;
        }
      }),
    ])
    .join('\n');
}

async function runFilterOracle(scenarios: readonly FilterScenario[]): Promise<number[]> {
  const stdout = await runBoundedCommand({
    arguments: [],
    command: FILTER_ORACLE_EXECUTABLE,
    input: serializeFilterScenarios(scenarios),
    label: `reSID oracle ${basename(FILTER_ORACLE_EXECUTABLE)}`,
    maximumOutputBytes: 16 * 1024 * 1024,
  });
  const output = stdout.trim();
  return output.length === 0 ? [] : output.split(/\s+/u).map(Number);
}

function runTypeScriptFilter(scenarios: readonly FilterScenario[]): number[] {
  const output: number[] = [];
  for (const scenario of scenarios) {
    const filter = new SidFilter(scenario.model, 1_000_000);
    let voices: readonly [number, number, number] = [0, 0, 0];
    for (const command of scenario.commands) {
      switch (command.kind) {
        case 'clock':
          for (let cycle = 0; cycle < command.cycles; cycle += 1) {
            filter.clock(voices[0], voices[1], voices[2]);
            output.push(filter.outputPcm);
          }
          break;
        case 'cutoff':
          filter.cutoff = command.cutoff;
          break;
        case 'modeVolume':
          filter.modeVolume = command.value;
          break;
        case 'reset':
          filter.reset();
          voices = [0, 0, 0];
          break;
        case 'resonanceRouting':
          filter.resonanceRouting = command.value;
          break;
        case 'voices':
          voices = command.values;
          break;
      }
    }
  }
  return output;
}

function filterScenarioAtSample(sample: number): { readonly cycle: number; readonly name: string } {
  let firstSample = 0;
  for (const scenario of FILTER_SCENARIOS) {
    const sampleCount = scenario.commands.reduce(
      (total, command) => total + (command.kind === 'clock' ? command.cycles : 0),
      0,
    );
    if (sample < firstSample + sampleCount) {
      return { cycle: sample - firstSample + 1, name: scenario.name };
    }
    firstSample += sampleCount;
  }
  throw new RangeError(`Filter sample ${sample} is outside all scenarios.`);
}

async function verifyFilterScenarios(): Promise<number> {
  const reference = await runFilterOracle(FILTER_SCENARIOS);
  const actual = runTypeScriptFilter(FILTER_SCENARIOS);
  if (actual.length !== reference.length) {
    throw new Error(
      `TypeScript produced ${actual.length} filter samples, reSID produced ${reference.length}.`,
    );
  }
  for (let sample = 0; sample < reference.length; sample += 1) {
    if (actual[sample] === reference[sample]) continue;
    const location = filterScenarioAtSample(sample);
    throw new Error(
      `${location.name}: filter mismatch at sampled cycle ${location.cycle}; TypeScript=${String(actual[sample])}, reSID=${String(reference[sample])}.`,
    );
  }
  return actual.length;
}

function serializeExternalFilterScenarios(scenarios: readonly ExternalFilterScenario[]): string {
  return scenarios
    .flatMap((scenario) =>
      scenario.commands.map((command) => {
        switch (command.kind) {
          case 'clock':
            return `CLOCK ${command.cycles}`;
          case 'input':
            return `INPUT ${command.value}`;
          case 'reset':
            return 'RESET';
        }
      }),
    )
    .join('\n');
}

async function runExternalFilterOracle(
  scenarios: readonly ExternalFilterScenario[],
): Promise<number[]> {
  const stdout = await runBoundedCommand({
    arguments: [],
    command: EXTERNAL_FILTER_ORACLE_EXECUTABLE,
    input: serializeExternalFilterScenarios(scenarios),
    label: `reSID oracle ${basename(EXTERNAL_FILTER_ORACLE_EXECUTABLE)}`,
    maximumOutputBytes: 16 * 1024 * 1024,
  });
  const output = stdout.trim();
  return output.length === 0 ? [] : output.split(/\s+/u).map(Number);
}

function runTypeScriptExternalFilter(scenarios: readonly ExternalFilterScenario[]): number[] {
  const filter = new SidExternalFilter(1_000_000);
  const output: number[] = [];
  let input = 0;
  for (const scenario of scenarios) {
    for (const command of scenario.commands) {
      switch (command.kind) {
        case 'clock':
          for (let cycle = 0; cycle < command.cycles; cycle += 1) {
            output.push(filter.clock(input));
          }
          break;
        case 'input':
          input = command.value;
          break;
        case 'reset':
          filter.reset();
          input = 0;
          break;
      }
    }
  }
  return output;
}

function externalFilterScenarioAtSample(sample: number): {
  readonly cycle: number;
  readonly name: string;
} {
  let firstSample = 0;
  for (const scenario of EXTERNAL_FILTER_SCENARIOS) {
    const sampleCount = scenario.commands.reduce(
      (total, command) => total + (command.kind === 'clock' ? command.cycles : 0),
      0,
    );
    if (sample < firstSample + sampleCount) {
      return { cycle: sample - firstSample + 1, name: scenario.name };
    }
    firstSample += sampleCount;
  }
  throw new RangeError(`External-filter sample ${sample} is outside all scenarios.`);
}

async function verifyExternalFilterScenarios(): Promise<number> {
  const reference = await runExternalFilterOracle(EXTERNAL_FILTER_SCENARIOS);
  const actual = runTypeScriptExternalFilter(EXTERNAL_FILTER_SCENARIOS);
  if (actual.length !== reference.length) {
    throw new Error(
      `TypeScript produced ${actual.length} external-filter samples, reSID produced ${reference.length}.`,
    );
  }
  for (let sample = 0; sample < reference.length; sample += 1) {
    if (actual[sample] === reference[sample]) continue;
    const location = externalFilterScenarioAtSample(sample);
    throw new Error(
      `${location.name}: external-filter mismatch at sampled cycle ${location.cycle}; TypeScript=${String(actual[sample])}, reSID=${String(reference[sample])}.`,
    );
  }
  return actual.length;
}

async function main(): Promise<void> {
  await buildOracles();
  let verifiedEnvelopeCycles = 0;
  for (const scenario of ENVELOPE_SCENARIOS) {
    verifiedEnvelopeCycles += await verifyScenario(scenario);
  }
  let verifiedOscillatorSamples = 0;
  for (const scenario of OSCILLATOR_SCENARIOS) {
    verifiedOscillatorSamples += await verifyOscillatorScenario(scenario);
  }
  const verifiedFilterSamples = await verifyFilterScenarios();
  const verifiedExternalFilterSamples = await verifyExternalFilterScenarios();
  console.log(
    `PASS reSID SID oracle (${RESID_REVISION.slice(0, 12)}): ${ENVELOPE_SCENARIOS.length} envelope scenarios / ${verifiedEnvelopeCycles.toLocaleString('en-US')} cycles; ${OSCILLATOR_SCENARIOS.length} oscillator scenarios / ${verifiedOscillatorSamples.toLocaleString('en-US')} samples; ${FILTER_SCENARIOS.length} MOS 6581/8580 filter scenarios / ${verifiedFilterSamples.toLocaleString('en-US')} samples; ${EXTERNAL_FILTER_SCENARIOS.length} board external-filter scenarios / ${verifiedExternalFilterSamples.toLocaleString('en-US')} samples.`,
  );
}

await main();
