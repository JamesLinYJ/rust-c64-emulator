// +-------------------------------------------------------------------------
//
//   TypeScript Commodore 64 模拟器 - 浏览器界面与 PRG 画面验证器
//
//   文件:       verifyBrowser.ts
//
//   日期:       2026年08月09日
//   作者:       OpenAI Codex
// --------------------------------------------------------------------------

import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';

import { chromium, type ConsoleMessage, type Page, type Response } from 'playwright';
import { PNG } from 'pngjs';
import { preview, type PreviewServer } from 'vite';

interface CanvasFrame {
  readonly data: Uint8Array;
  readonly height: number;
  readonly width: number;
}

interface AudioLifecycleProbe {
  bufferSourceCreations: number;
  clears: number;
  sampleBatches: number;
}

interface AudioProbeWindow extends Window {
  __audioLifecycleProbe?: AudioLifecycleProbe;
}

interface BrowserRenderMetrics {
  readonly framesPerSecond: number;
  readonly overBudgetFrames: number;
  readonly p95Ms: number;
  readonly sampledFrames: number;
}

interface BrowserProblemAllowlist {
  readonly console?: (message: ConsoleMessage) => boolean;
  readonly response?: (response: Response) => boolean;
}

const PREVIEW_URL = 'http://127.0.0.1:4173';
const OUTPUT_DIRECTORY = resolve('output/playwright/reference-ui');
const PROGRAM_START_TIMEOUT_MS = 10_000;
const PROGRAM_COUNTER_PATTERN = /PC \$([0-9A-F]{4})/u;
const RENDER_METRICS_PATTERN = /呈现\s+(\d+)\s+FPS.*?p95\s+([\d.]+)\s+ms.*?超预算\s+(\d+)\/(\d+)/u;
const AUDIO_METRICS_PATTERN = /音频欠载\s+(\d+)\s+·\s+音频溢出\s+(\d+)/u;
const PNG_DATA_URL_PREFIX = 'data:image/png;base64,';
const RGBA_BYTE_COUNT = 4;
const MINIMUM_PROGRAM_CHANGED_PIXELS = 4_000;
const MINIMUM_PROGRAM_COLOR_COUNT = 2;
const MINIMUM_TOUCH_TARGET_PX = 44;
const MINIMUM_MOBILE_CANVAS_WIDTH_PX = 320;
const MINIMUM_NTSC_FRAMES_PER_SECOND = 55;
const MAXIMUM_NTSC_FRAMES_PER_SECOND = 65;
const BASIC_FIRMWARE_PATH = '/firmware/basic.901226-01.bin';
const GALAGA_PROGRAM_PATH = '/programs/galaga.prg';
const INITIALIZATION_HTTP_ERROR = '模拟器固件下载失败（HTTP 503）。请稍后重试。';
const DESKTOP_VIEWPORT = { height: 900, width: 1_440 } as const;
const MOBILE_PORTRAIT_VIEWPORT = { height: 844, width: 390 } as const;
const MOBILE_LANDSCAPE_VIEWPORT = { height: 390, width: 844 } as const;

async function installAudioLifecycleProbe(page: Page): Promise<void> {
  await page.evaluate(() => {
    const probe: AudioLifecycleProbe = {
      bufferSourceCreations: 0,
      clears: 0,
      sampleBatches: 0,
    };
    (window as AudioProbeWindow).__audioLifecycleProbe = probe;
    Reflect.set(AudioContext.prototype, 'createBufferSource', () => {
      probe.bufferSourceCreations += 1;
      throw new Error('Legacy AudioBufferSourceNode scheduling is not allowed.');
    });

    const postMessage: unknown = Reflect.get(MessagePort.prototype, 'postMessage');
    if (typeof postMessage !== 'function')
      throw new Error('MessagePort.postMessage is unavailable.');
    Reflect.set(
      MessagePort.prototype,
      'postMessage',
      function (
        this: MessagePort,
        message: unknown,
        transferOrOptions?: StructuredSerializeOptions | Transferable[],
      ): void {
        if (typeof message === 'object' && message !== null) {
          const type: unknown = Reflect.get(message, 'type');
          if (type === 'samples') probe.sampleBatches += 1;
          else if (type === 'clear') probe.clears += 1;
        }
        Reflect.apply(
          postMessage,
          this,
          transferOrOptions === undefined ? [message] : [message, transferOrOptions],
        );
      },
    );
  });
}

async function readAudioLifecycleProbe(page: Page): Promise<AudioLifecycleProbe> {
  return page.evaluate(() => {
    const probe = (window as AudioProbeWindow).__audioLifecycleProbe;
    if (!probe) throw new Error('Audio lifecycle probe was not installed.');
    return { ...probe };
  });
}

async function readBrowserRenderMetrics(page: Page): Promise<BrowserRenderMetrics> {
  const text = (await page.locator('[aria-label="实时执行数据"]').textContent()) ?? '';
  const match = RENDER_METRICS_PATTERN.exec(text);
  if (!match)
    throw new Error(`Runtime telemetry did not contain render metrics: "${text.trim()}".`);

  const framesPerSecond = Number.parseInt(match[1] ?? '', 10);
  const p95Ms = Number.parseFloat(match[2] ?? '');
  const overBudgetFrames = Number.parseInt(match[3] ?? '', 10);
  const sampledFrames = Number.parseInt(match[4] ?? '', 10);
  if (
    !Number.isFinite(framesPerSecond) ||
    framesPerSecond <= 0 ||
    !Number.isFinite(p95Ms) ||
    p95Ms < 0 ||
    !Number.isSafeInteger(overBudgetFrames) ||
    overBudgetFrames < 0 ||
    !Number.isSafeInteger(sampledFrames) ||
    sampledFrames <= 0 ||
    overBudgetFrames > sampledFrames
  ) {
    throw new Error(`Runtime telemetry contained invalid render metrics: "${text.trim()}".`);
  }
  return { framesPerSecond, overBudgetFrames, p95Ms, sampledFrames };
}

async function verifyZeroAudioStreamErrors(page: Page, videoStandard: string): Promise<void> {
  const text = (await page.locator('[aria-label="实时执行数据"]').textContent()) ?? '';
  const match = AUDIO_METRICS_PATTERN.exec(text);
  if (!match) throw new Error(`${videoStandard} telemetry did not publish audio stream metrics.`);
  const underrunSamples = Number.parseInt(match[1] ?? '', 10);
  const overrunSamples = Number.parseInt(match[2] ?? '', 10);
  if (underrunSamples !== 0 || overrunSamples !== 0) {
    throw new Error(
      `${videoStandard} audio reported ${underrunSamples} underrun and ` +
        `${overrunSamples} overrun samples.`,
    );
  }
}

async function waitForProgramExecution(page: Page): Promise<number> {
  const executionData = page.locator('[aria-label="实时执行数据"]');
  const deadline = Date.now() + PROGRAM_START_TIMEOUT_MS;
  let latestText = '';

  while (Date.now() < deadline) {
    latestText = (await executionData.textContent()) ?? '';
    const match = PROGRAM_COUNTER_PATTERN.exec(latestText);
    const programCounter = match?.[1] === undefined ? Number.NaN : Number.parseInt(match[1], 16);
    if (programCounter >= 0x0801 && programCounter < 0xa000) return programCounter;
    await page.waitForTimeout(100);
  }

  throw new Error(`Galaga did not begin executing from RAM; latest status was "${latestText}".`);
}

function collectBrowserProblems(
  page: Page,
  problems: string[],
  allowlist: BrowserProblemAllowlist = {},
): void {
  const previewOrigin = new URL(PREVIEW_URL).origin;
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') {
      if (allowlist.console?.(message)) return;
      problems.push(`console.${message.type()}: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => problems.push(`pageerror: ${error.message}`));
  page.on('requestfailed', (request) => {
    if (new URL(request.url()).origin !== previewOrigin) return;
    problems.push(
      `requestfailed: ${request.method()} ${request.url()} (${request.failure()?.errorText ?? 'unknown error'})`,
    );
  });
  page.on('response', (response) => {
    if (response.status() < 400 || new URL(response.url()).origin !== previewOrigin) return;
    if (allowlist.response?.(response)) return;
    problems.push(
      `response ${response.status()}: ${response.request().method()} ${response.url()}`,
    );
  });
}

async function verifyInitializationError(
  page: Page,
  viewportHeight: number,
  attempt: string,
): Promise<void> {
  const alert = page.getByRole('alert');
  await alert.waitFor();
  const message = (await alert.textContent())?.trim();
  if (message !== INITIALIZATION_HTTP_ERROR) {
    throw new Error(`${attempt} initialization failure showed "${message ?? ''}".`);
  }
  if (message.includes('Failed to fetch') || message.includes('Unable to load')) {
    throw new Error(`${attempt} initialization failure leaked an English transport error.`);
  }
  const errorColors = await page.evaluate(() => {
    const alertElement = document.querySelector<HTMLElement>('[role="alert"]');
    const machineStatus = document.querySelector<HTMLElement>('.machine-status--error');
    return {
      alert: alertElement ? getComputedStyle(alertElement).color : '',
      status: machineStatus ? getComputedStyle(machineStatus).color : '',
    };
  });
  if (!errorColors.alert || errorColors.status !== errorColors.alert) {
    throw new Error(`${attempt} machine status did not use the visible error color.`);
  }

  const retry = page.getByRole('button', { name: '重新初始化' });
  await retry.waitFor();
  const bounds = await retry.boundingBox();
  if (!bounds) throw new Error(`${attempt} retry control is not visible.`);
  if (bounds.width < MINIMUM_TOUCH_TARGET_PX || bounds.height < MINIMUM_TOUCH_TARGET_PX) {
    throw new Error(
      `${attempt} retry control is only ${bounds.width.toFixed(1)}x${bounds.height.toFixed(1)}px.`,
    );
  }
  if (bounds.y < 0 || bounds.y + bounds.height > viewportHeight) {
    throw new Error(`${attempt} retry control is outside the initial mobile viewport.`);
  }
}

async function verifyMobileInitializationRecovery(
  page: Page,
  expectedFailures: { acceptedConsoleErrors: number; acceptedResponses: number },
  requestCount: () => number,
): Promise<void> {
  await page.goto(PREVIEW_URL, { waitUntil: 'networkidle' });
  await verifyInitializationError(page, MOBILE_PORTRAIT_VIEWPORT.height, 'First');
  await verifyNoHorizontalOverflow(page, 'Mobile recovery error');
  await page.screenshot({
    path: resolve(OUTPUT_DIRECTORY, 'current-mobile-recovery-error.png'),
    fullPage: false,
  });

  const retry = page.getByRole('button', { name: '重新初始化' });
  await retry.focus();
  const secondFailure = page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname.endsWith(BASIC_FIRMWARE_PATH) && response.status() === 503,
  );
  await page.keyboard.press('Enter');
  const secondFailureResponse = await secondFailure;
  await secondFailureResponse.finished();
  await verifyInitializationError(page, MOBILE_PORTRAIT_VIEWPORT.height, 'Second');

  const successfulFirmware = page.waitForResponse(
    (response) => new URL(response.url()).pathname.endsWith(BASIC_FIRMWARE_PATH) && response.ok(),
  );
  await page.getByRole('button', { name: '重新初始化' }).click();
  const successfulFirmwareResponse = await successfulFirmware;
  await successfulFirmwareResponse.finished();
  await waitForBoot(page);
  if ((await page.getByRole('button', { name: '重新初始化' }).count()) !== 0) {
    throw new Error('Retry control remained after successful initialization.');
  }
  if ((await page.locator('canvas').count()) !== 1) {
    throw new Error('Initialization recovery duplicated the emulator canvas.');
  }
  if (requestCount() !== 3 || expectedFailures.acceptedResponses !== 2) {
    throw new Error(
      `Recovery used ${requestCount()} BASIC requests and allowed ` +
        `${expectedFailures.acceptedResponses} expected 503 responses; expected 3 and 2.`,
    );
  }
  if (expectedFailures.acceptedConsoleErrors > 2) {
    throw new Error('Recovery allowed more console errors than the two expected HTTP failures.');
  }
  await verifyNoHorizontalOverflow(page, 'Mobile recovery success');
  await verifyMinimumTouchTargets(page, 'Mobile recovery success');
}

async function waitForBoot(page: Page): Promise<void> {
  await page.locator('.boot-overlay.is-complete').waitFor({ state: 'attached', timeout: 20_000 });
  await page.waitForFunction(() => {
    const overlay = document.querySelector('.boot-overlay');
    if (!overlay?.classList.contains('is-complete')) return false;
    const style = getComputedStyle(overlay);
    return style.opacity === '0' && style.visibility === 'hidden';
  });
}

async function verifyNoHorizontalOverflow(page: Page, viewportName: string): Promise<void> {
  const overflow = await page.evaluate(() => {
    const viewportWidth = document.documentElement.clientWidth;
    const contentWidth = Math.max(
      document.documentElement.scrollWidth,
      document.body?.scrollWidth ?? 0,
    );
    return contentWidth - viewportWidth;
  });
  if (overflow > 1) {
    throw new Error(`${viewportName} layout overflows horizontally by ${overflow}px.`);
  }
}

async function verifyMinimumTouchTargets(page: Page, viewportName: string): Promise<void> {
  const undersized = await page
    .locator('button, select, summary, a[href], [tabindex="0"]')
    .evaluateAll(
      (elements, minimumSize) =>
        elements.flatMap((element) => {
          const bounds = element.getBoundingClientRect();
          const style = getComputedStyle(element);
          if (
            bounds.width === 0 ||
            bounds.height === 0 ||
            style.display === 'none' ||
            style.visibility === 'hidden'
          ) {
            return [];
          }
          if (bounds.width >= minimumSize && bounds.height >= minimumSize) return [];
          const label =
            element.getAttribute('aria-label') ??
            element.textContent?.trim().replace(/\s+/gu, ' ').slice(0, 64) ??
            element.tagName.toLowerCase();
          return [`${label}: ${bounds.width.toFixed(1)}x${bounds.height.toFixed(1)}px`];
        }),
      MINIMUM_TOUCH_TARGET_PX,
    );
  if (undersized.length > 0) {
    throw new Error(
      `${viewportName} has controls smaller than ${MINIMUM_TOUCH_TARGET_PX}px:\n${undersized.join('\n')}`,
    );
  }
}

async function captureCanvasFrame(page: Page): Promise<CanvasFrame> {
  const dataUrl = await page.locator('canvas').evaluate((element) => {
    if (!(element instanceof HTMLCanvasElement)) {
      throw new TypeError('The emulator screen is not an HTML canvas element.');
    }
    return element.toDataURL('image/png');
  });
  if (!dataUrl.startsWith(PNG_DATA_URL_PREFIX)) {
    throw new Error('The emulator canvas did not produce a PNG data URL.');
  }
  const decoded = PNG.sync.read(Buffer.from(dataUrl.slice(PNG_DATA_URL_PREFIX.length), 'base64'));
  return { data: decoded.data, height: decoded.height, width: decoded.width };
}

function frameByte(frame: CanvasFrame, offset: number): number {
  const value = frame.data[offset];
  if (value === undefined) {
    throw new RangeError(`Canvas frame byte ${offset} is outside its decoded pixel data.`);
  }
  return value;
}

function verifyProgramCanvas(before: CanvasFrame, after: CanvasFrame): void {
  if (before.width !== after.width || before.height !== after.height) {
    throw new Error(
      `Program canvas changed dimensions from ${before.width}x${before.height} ` +
        `to ${after.width}x${after.height}.`,
    );
  }

  const expectedBytes = after.width * after.height * RGBA_BYTE_COUNT;
  if (before.data.length !== expectedBytes || after.data.length !== expectedBytes) {
    throw new Error(
      `Decoded canvas data does not match ${after.width}x${after.height} RGBA dimensions.`,
    );
  }

  let changedPixels = 0;
  const colors = new Set<number>();
  for (let offset = 0; offset < expectedBytes; offset += RGBA_BYTE_COUNT) {
    const red = frameByte(after, offset);
    const green = frameByte(after, offset + 1);
    const blue = frameByte(after, offset + 2);
    colors.add((red << 16) | (green << 8) | blue);

    if (
      red !== frameByte(before, offset) ||
      green !== frameByte(before, offset + 1) ||
      blue !== frameByte(before, offset + 2)
    ) {
      changedPixels += 1;
    }
  }

  if (changedPixels < MINIMUM_PROGRAM_CHANGED_PIXELS) {
    throw new Error(
      `Galaga changed only ${changedPixels} canvas pixels after loading; ` +
        `expected at least ${MINIMUM_PROGRAM_CHANGED_PIXELS}.`,
    );
  }
  if (colors.size < MINIMUM_PROGRAM_COLOR_COUNT) {
    throw new Error(
      `Galaga canvas contains ${colors.size} color; expected at least ${MINIMUM_PROGRAM_COLOR_COUNT}.`,
    );
  }
}

async function verifyDesktop(page: Page): Promise<BrowserRenderMetrics> {
  await page.goto(PREVIEW_URL, { waitUntil: 'networkidle' });
  await waitForBoot(page);
  await installAudioLifecycleProbe(page);
  await verifyNoHorizontalOverflow(page, 'Desktop');

  await page.getByRole('heading', { name: 'Commodore 64' }).waitFor();
  await page.getByRole('heading', { name: '快捷载入 PRG' }).waitFor();
  await page.getByRole('heading', { name: '运行控制' }).waitFor();
  await page.getByRole('heading', { name: '快捷键参考' }).waitFor();

  await page.getByRole('button', { name: '启用声音' }).click();
  await page.getByText('声音已开启', { exact: true }).waitFor();
  await page.waitForFunction(
    () => ((window as AudioProbeWindow).__audioLifecycleProbe?.sampleBatches ?? 0) > 0,
  );
  await page.getByRole('button', { name: '暂停' }).click();
  await page.getByText('已暂停', { exact: true }).first().waitFor();
  await page.waitForFunction(
    () => ((window as AudioProbeWindow).__audioLifecycleProbe?.clears ?? 0) > 0,
  );
  const pausedAudio = await readAudioLifecycleProbe(page);
  if (pausedAudio.bufferSourceCreations !== 0) {
    throw new Error(`Audio created ${pausedAudio.bufferSourceCreations} buffer source(s).`);
  }
  await page.getByRole('button', { name: '单帧' }).click();
  await page.waitForTimeout(50);
  const steppedAudio = await readAudioLifecycleProbe(page);
  if (steppedAudio.sampleBatches !== pausedAudio.sampleBatches) {
    throw new Error('Paused single-frame execution scheduled non-realtime browser audio.');
  }
  const basicReadyFrame = await captureCanvasFrame(page);
  await page.getByRole('button', { name: '运行' }).click();
  await page.getByText('运行中', { exact: true }).first().waitFor();
  await page.waitForFunction(
    (previousBatchCount) =>
      ((window as AudioProbeWindow).__audioLifecycleProbe?.sampleBatches ?? 0) > previousBatchCount,
    pausedAudio.sampleBatches,
  );

  await page.getByRole('button', { name: '切换到深色主题' }).click();
  await page.locator('.app-page--dark').waitFor();
  await page.getByRole('button', { name: '切换到浅色主题' }).click();

  await page.getByRole('button', { name: '载入程序' }).click();
  await page.getByText(/已载入 .* 字节至 \$0801/).waitFor({ timeout: 10_000 });
  await waitForProgramExecution(page);
  await page.waitForTimeout(2_000);
  const renderMetrics = await readBrowserRenderMetrics(page);
  await verifyZeroAudioStreamErrors(page, 'PAL');
  await page.getByRole('button', { name: '暂停' }).click();
  verifyProgramCanvas(basicReadyFrame, await captureCanvasFrame(page));
  await page.getByRole('button', { name: '运行' }).click();

  await page.screenshot({
    path: resolve(OUTPUT_DIRECTORY, 'current-desktop.png'),
    fullPage: true,
  });

  const beforeResetAudio = await readAudioLifecycleProbe(page);
  await page.getByRole('button', { name: '重置' }).click();
  await page.waitForFunction(
    (previousClearCount) =>
      ((window as AudioProbeWindow).__audioLifecycleProbe?.clears ?? 0) > previousClearCount,
    beforeResetAudio.clears,
  );
  await page.waitForFunction(
    (previousBatchCount) =>
      ((window as AudioProbeWindow).__audioLifecycleProbe?.sampleBatches ?? 0) > previousBatchCount,
    beforeResetAudio.sampleBatches,
  );
  return renderMetrics;
}

async function verifyNtscRuntime(page: Page): Promise<BrowserRenderMetrics> {
  await page.goto(PREVIEW_URL, { waitUntil: 'networkidle' });
  await waitForBoot(page);
  await installAudioLifecycleProbe(page);
  await page.getByRole('button', { name: 'NTSC', exact: true }).click();
  await page.waitForFunction(() => {
    const canvas = document.querySelector('canvas');
    return canvas instanceof HTMLCanvasElement && canvas.width === 403 && canvas.height === 247;
  });
  await waitForBoot(page);
  await page.getByText('NTSC · Rust/Wasm Worker 模拟器', { exact: true }).waitFor();
  await page.getByRole('button', { name: '启用声音' }).click();
  await page.getByText('声音已开启', { exact: true }).waitFor();
  await page.waitForFunction(
    () => ((window as AudioProbeWindow).__audioLifecycleProbe?.sampleBatches ?? 0) > 0,
  );
  await page.waitForFunction(() => {
    const telemetry = document.querySelector('[aria-label="实时执行数据"]')?.textContent ?? '';
    return /NTSC 59\.83 Hz.*呈现\s+\d+\s+FPS.*超预算\s+\d+\/\d+/u.test(telemetry);
  });
  await page.waitForTimeout(2_500);

  const metrics = await readBrowserRenderMetrics(page);
  await verifyZeroAudioStreamErrors(page, 'NTSC');
  if (
    metrics.framesPerSecond < MINIMUM_NTSC_FRAMES_PER_SECOND ||
    metrics.framesPerSecond > MAXIMUM_NTSC_FRAMES_PER_SECOND
  ) {
    throw new Error(
      `NTSC runtime produced ${metrics.framesPerSecond} FPS; expected ` +
        `${MINIMUM_NTSC_FRAMES_PER_SECOND}-${MAXIMUM_NTSC_FRAMES_PER_SECOND}.`,
    );
  }
  await verifyNoHorizontalOverflow(page, 'NTSC desktop');
  await page.screenshot({
    path: resolve(OUTPUT_DIRECTORY, 'current-desktop-ntsc.png'),
    fullPage: true,
  });
  return metrics;
}

async function verifyBundledProgramIntegrity(page: Page): Promise<void> {
  await page.route(`**${GALAGA_PROGRAM_PATH}`, async (route) => {
    const response = await route.fetch();
    const changed = await response.body();
    const lastByte = changed.length - 1;
    if (lastByte < 2) throw new Error('Galaga integrity probe received an invalid PRG payload.');
    changed[lastByte] = (changed[lastByte] ?? 0) ^ 0x01;
    await route.fulfill({ response, body: changed });
  });

  await page.goto(PREVIEW_URL, { waitUntil: 'networkidle' });
  await waitForBoot(page);
  await page.getByRole('button', { name: '载入程序' }).click();
  const alert = page.getByRole('alert');
  await alert.waitFor();
  const message = (await alert.textContent()) ?? '';
  if (!/Galaga\.prg|galaga\.prg/iu.test(message) || !/SHA-256 mismatch/iu.test(message)) {
    throw new Error(`Changed bundled PRG produced an unexpected error: "${message.trim()}".`);
  }
  await page.unroute(`**${GALAGA_PROGRAM_PATH}`);
}

async function verifyTouchInput(page: Page): Promise<void> {
  const dpad = page.getByRole('group', { name: '八方向控制' });
  const fire = page.getByRole('button', { name: '开火' });
  const dpadBounds = await dpad.boundingBox();
  const fireBounds = await fire.boundingBox();
  if (!dpadBounds || !fireBounds) {
    throw new Error('Touch controls disappeared before input testing.');
  }
  const upLeftPoint = {
    force: 1,
    id: 1,
    radiusX: 8,
    radiusY: 8,
    x: dpadBounds.x + dpadBounds.width * 0.2,
    y: dpadBounds.y + dpadBounds.height * 0.2,
  };
  const upRightPoint = {
    ...upLeftPoint,
    x: dpadBounds.x + dpadBounds.width * 0.8,
  };
  const firePoint = {
    force: 1,
    id: 2,
    radiusX: 8,
    radiusY: 8,
    x: fireBounds.x + fireBounds.width / 2,
    y: fireBounds.y + fireBounds.height / 2,
  };
  const session = await page.context().newCDPSession(page);

  await session.send('Input.dispatchTouchEvent', {
    touchPoints: [upLeftPoint],
    type: 'touchStart',
  });
  await page.waitForFunction(
    () =>
      document.querySelector('[data-joystick-control="up"]')?.getAttribute('aria-pressed') ===
        'true' &&
      document.querySelector('[data-joystick-control="left"]')?.getAttribute('aria-pressed') ===
        'true',
  );

  await session.send('Input.dispatchTouchEvent', {
    touchPoints: [upLeftPoint, firePoint],
    type: 'touchStart',
  });
  await page.waitForFunction(
    () =>
      document.querySelector('[data-joystick-control="up"]')?.getAttribute('aria-pressed') ===
        'true' &&
      document.querySelector('[data-joystick-control="fire"]')?.getAttribute('aria-pressed') ===
        'true',
  );

  await session.send('Input.dispatchTouchEvent', {
    touchPoints: [upRightPoint, firePoint],
    type: 'touchMove',
  });
  await page.waitForFunction(
    () =>
      document.querySelector('[data-joystick-control="up"]')?.getAttribute('aria-pressed') ===
        'true' &&
      document.querySelector('[data-joystick-control="left"]')?.getAttribute('aria-pressed') ===
        'false' &&
      document.querySelector('[data-joystick-control="right"]')?.getAttribute('aria-pressed') ===
        'true' &&
      document.querySelector('[data-joystick-control="fire"]')?.getAttribute('aria-pressed') ===
        'true',
  );

  // 当前 Chromium 的 CDP touchEnd 以 touchPoints 指明本次结束的触点。
  await session.send('Input.dispatchTouchEvent', {
    touchPoints: [firePoint],
    type: 'touchEnd',
  });
  await page.waitForFunction(
    () =>
      document.querySelector('[data-joystick-control="up"]')?.getAttribute('aria-pressed') ===
        'true' &&
      document.querySelector('[data-joystick-control="right"]')?.getAttribute('aria-pressed') ===
        'true' &&
      document.querySelector('[data-joystick-control="fire"]')?.getAttribute('aria-pressed') ===
        'false',
  );

  await session.send('Input.dispatchTouchEvent', {
    touchPoints: [upRightPoint],
    type: 'touchEnd',
  });
  await page.waitForFunction(
    () =>
      document.querySelector('[data-joystick-control="up"]')?.getAttribute('aria-pressed') ===
        'false' &&
      document.querySelector('[data-joystick-control="right"]')?.getAttribute('aria-pressed') ===
        'false',
  );
}

async function verifyMobile(
  page: Page,
  viewportName: string,
  viewportHeight: number,
  screenshotName: string,
  options: { readonly input: boolean; readonly initiallyVisible: boolean },
): Promise<void> {
  await page.goto(PREVIEW_URL, { waitUntil: 'networkidle' });
  await waitForBoot(page);
  await page.getByRole('button', { name: '启用声音' }).waitFor();
  await verifyNoHorizontalOverflow(page, viewportName);
  await verifyMinimumTouchTargets(page, viewportName);

  const touchControls = page.getByRole('region', { name: '触控操纵杆' });
  await touchControls.waitFor();
  const touchBounds = await touchControls.boundingBox();
  if (!touchBounds) throw new Error(`${viewportName} touch controls are not visible.`);
  if (options.initiallyVisible && touchBounds.y + touchBounds.height > viewportHeight) {
    throw new Error(
      `${viewportName} touch controls are not fully available in the initial viewport.`,
    );
  }

  const canvasBounds = await page.locator('canvas').boundingBox();
  if (!canvasBounds || canvasBounds.width < MINIMUM_MOBILE_CANVAS_WIDTH_PX) {
    throw new Error(
      `Mobile canvas is ${canvasBounds?.width ?? 0}px wide; ` +
        `expected at least ${MINIMUM_MOBILE_CANVAS_WIDTH_PX}px.`,
    );
  }

  if (options.input) await verifyTouchInput(page);

  if (!options.initiallyVisible) await touchControls.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: resolve(OUTPUT_DIRECTORY, screenshotName),
    fullPage: options.initiallyVisible,
  });

  const postCapture = await touchControls.evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    return {
      bottom: bounds.bottom,
      coarsePointer: matchMedia('(hover: none) and (pointer: coarse)').matches,
      display: getComputedStyle(element).display,
      top: bounds.top,
    };
  });
  if (
    postCapture.display === 'none' ||
    (!options.initiallyVisible && !postCapture.coarsePointer) ||
    postCapture.top < 0 ||
    postCapture.bottom > viewportHeight
  ) {
    throw new Error(
      `${viewportName} touch controls changed during capture: ${JSON.stringify(postCapture)}.`,
    );
  }
}

async function main(): Promise<void> {
  await mkdir(OUTPUT_DIRECTORY, { recursive: true });
  const previewServer = await preview({
    preview: { host: '127.0.0.1', port: 4173, strictPort: true },
  });
  const browser = await chromium.launch({ headless: true });
  const problems: string[] = [];

  try {
    const desktop = await browser.newPage({ viewport: DESKTOP_VIEWPORT });
    collectBrowserProblems(desktop, problems);
    const desktopMetrics = await verifyDesktop(desktop);
    await desktop.close();

    const ntsc = await browser.newPage({ viewport: DESKTOP_VIEWPORT });
    collectBrowserProblems(ntsc, problems);
    const ntscMetrics = await verifyNtscRuntime(ntsc);
    await ntsc.close();

    const integrity = await browser.newPage({ viewport: DESKTOP_VIEWPORT });
    collectBrowserProblems(integrity, problems);
    await verifyBundledProgramIntegrity(integrity);
    await integrity.close();

    const mobile = await browser.newPage({
      hasTouch: true,
      isMobile: true,
      viewport: MOBILE_PORTRAIT_VIEWPORT,
    });
    collectBrowserProblems(mobile, problems);
    await verifyMobile(
      mobile,
      'Mobile portrait',
      MOBILE_PORTRAIT_VIEWPORT.height,
      'current-mobile.png',
      { initiallyVisible: true, input: true },
    );
    await mobile.close();

    const landscape = await browser.newPage({
      hasTouch: true,
      isMobile: true,
      viewport: MOBILE_LANDSCAPE_VIEWPORT,
    });
    collectBrowserProblems(landscape, problems);
    await verifyMobile(
      landscape,
      'Mobile landscape',
      MOBILE_LANDSCAPE_VIEWPORT.height,
      'current-mobile-landscape.png',
      { initiallyVisible: false, input: false },
    );
    await landscape.close();

    const recovery = await browser.newPage({
      hasTouch: true,
      isMobile: true,
      viewport: MOBILE_PORTRAIT_VIEWPORT,
    });
    let basicFirmwareRequests = 0;
    const expectedFailures = { acceptedConsoleErrors: 0, acceptedResponses: 0 };
    await recovery.route(`**${BASIC_FIRMWARE_PATH}`, async (route) => {
      basicFirmwareRequests += 1;
      if (basicFirmwareRequests <= 2) {
        await route.fulfill({
          body: 'expected browser recovery probe',
          contentType: 'text/plain; charset=utf-8',
          status: 503,
        });
        return;
      }
      await route.continue();
    });
    collectBrowserProblems(recovery, problems, {
      console: (message) => {
        if (
          expectedFailures.acceptedConsoleErrors >= 2 ||
          !/status of 503|HTTP 503/iu.test(message.text())
        ) {
          return false;
        }
        expectedFailures.acceptedConsoleErrors += 1;
        return true;
      },
      response: (response) => {
        if (
          expectedFailures.acceptedResponses >= 2 ||
          response.status() !== 503 ||
          !new URL(response.url()).pathname.endsWith(BASIC_FIRMWARE_PATH)
        ) {
          return false;
        }
        expectedFailures.acceptedResponses += 1;
        return true;
      },
    });
    await verifyMobileInitializationRecovery(
      recovery,
      expectedFailures,
      () => basicFirmwareRequests,
    );
    await recovery.close();

    console.log(
      `Browser render metrics: ${desktopMetrics.framesPerSecond} FPS, ` +
        `p95 ${desktopMetrics.p95Ms.toFixed(2)} ms, ` +
        `${desktopMetrics.overBudgetFrames}/${desktopMetrics.sampledFrames} over budget; ` +
        `NTSC ${ntscMetrics.framesPerSecond} FPS, p95 ${ntscMetrics.p95Ms.toFixed(2)} ms, ` +
        `${ntscMetrics.overBudgetFrames}/${ntscMetrics.sampledFrames} over budget.`,
    );
  } finally {
    await browser.close();
    await closePreviewServer(previewServer);
  }

  if (problems.length > 0) {
    throw new Error(`Browser verification reported problems:\n${problems.join('\n')}`);
  }
  console.log(
    'PASS browser UI: PAL/NTSC realtime, 1440 desktop, 390 portrait/recovery, 844 landscape, CDP multi-touch, 44px targets, 0 unexpected overflow/warnings/errors.',
  );
}

async function closePreviewServer(server: PreviewServer): Promise<void> {
  await new Promise<void>((resolveClose, rejectClose) => {
    server.httpServer.close((error) => {
      if (error) rejectClose(error);
      else resolveClose();
    });
  });
}

await main();
