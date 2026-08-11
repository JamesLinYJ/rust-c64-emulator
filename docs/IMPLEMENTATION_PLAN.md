# Rust C64 Emulator 实施路线图

创建日期：2026年08月10日

维护者：OpenAI Codex

## 项目原则

- Strict、Turbo、SuperCPU、VM Enhanced 是公开的体系结构模式，不针对软件名称、镜像哈希或 benchmark 特判。
- Rust 拥有生产时序核心；TypeScript/React 负责平台 API、Worker、UI，并在迁移期间作为差分 oracle。
- 每条 6510 指令保留架构周期成本；Turbo 通过每个 C64 系统周期的内部槽位获得加速，外设仍运行在 PAL/NTSC 时钟域。
- 基础 64 KiB RAM 始终连贯；MMIO、处理器端口、颜色 RAM、Cartridge、REU/DMA 通过统一 BusBridge。
- 普通修改不改历史文件日期；新建或实质重构文件记录真实日期和实际作者。
- 暂不创建 GitHub Projects 看板。所有进度以本文件和本地测试结果为准，完整验证前不发布。
- Rust、Clippy、TypeScript、ESLint 与 Wasm 构建均以零 warning 为硬门槛，不使用全局 suppression 掩盖问题。
- Rust Strict 在成为生产后端前必须通过仓库现有全部固定版本、固定 SHA-256 的 VICE 参考验证。

## 当前基线

- [x] 创建公开仓库 `JamesLinYJ/rust-c64-emulator`。
- [x] 从 TypeScript 仓库迁移 `main`，保留原提交、作者和日期。
- [x] 保持旧仓库不改名、不归档；新仓库保留只读 `legacy` remote。
- [x] 创建本地开发分支 `agent/rust-wasm-modern-vm`。
- [x] 建立本地实施路线图。

## M0：Foundation

- [x] 建立 Cargo workspace：`c64-core`、`c64-wasm`。
- [x] 固定 Rust 1.97.0、edition 2024、`wasm32-unknown-unknown`、wasm-pack 0.15.0、wasm-bindgen 0.2.127。
- [x] 生成并纳入版本控制范围的 `Cargo.lock`，配置 release LTO、单 codegen unit 和 `panic=abort`。
- [x] 定义 PAL/NTSC 虚拟时间、profile、execution、pacing、BusBridge 和 save-state v1。
- [x] 为浏览器和 Node 生成同源 Wasm 包；禁止每周期跨 JS/Wasm。
- [x] 将 Cargo、Clippy、rustfmt 和 Wasm 构建加入现有 CI 与 npm 门禁。

验收：全新 checkout 可以构建 native、web Wasm、Node Wasm，并通过原 TypeScript 全部门禁。

验收证据（2026年08月10日）：`npm run check`、`npm run verify:wasm` 以及固定 SHA-256 的完整
`verify:reference` 链均通过；VICE revision 46176 的 CPU port、CIA、VIC-II、1541、Tape、SID、
Cartridge 与程序语料无差异。参考资产下载采用哈希校验、有界重试和原子缓存，网络瞬断不会再留下半文件。

## M1：Rust Strict

- [x] 迁移 6510、处理器端口、PLA、RAM/ROM 和精确总线周期。
- [x] 迁移 VIC-II、CIA、SID 和主机严格调度器。
- [x] 完成 1541 VIA2、磁盘机构、D64/G64、drive memory、独立 6502、整数时钟同步与 C64Chipset/IEC 整机迁移。
- [x] 迁移 Datasette、TAP/Writable TAP、6510 motor/write/sense/read 接线与粗粒度 Wasm 媒体 ABI。
- [x] 迁移 CRT、普通 8K/16K/Ultimax、Ocean、Magic Desk、EasyFlash 与 AM29F040B 状态机。
- [x] 迁移 REU。
- [x] 把版本化架构状态扩展到全部芯片、外设、媒体和指令中间周期，并保留 save-state v1 向后兼容。
- [x] 所有参考测试一致后切换生产 Strict；TypeScript 核心转为测试 oracle。

验收：全部 256 opcode、IRQ/NMI/RDY/BA/AEC、设备参考轨迹、程序、视频和音频零语义回归。

1541 验收证据（2026年08月11日）：VIA2、GCR、D64/G64、磁盘机构、2 KiB RAM/ROM 映射、
独立 6502、整数主机/驱动器时钟和 C64Chipset/IEC 均通过 TypeScript/Rust 确定性差分；完整
VICE revision 46176 `verify:drive` 通过目录、LOAD、SAVE、format、IEC delay、write-protect、
disk-change 与 HLS protection，D64 写回和 G64 可变速度图保持可重放。

Datasette 验收证据（2026年08月11日）：Rust KERNAL LOAD 消耗 43,734 个物理 READ 脉冲并在
`$C000-$C03F` 得到固定载荷；VICE revision 46176 `tap204060once.prg` 产生
`256/512/768/512` 周期尾波形；KERNAL SAVE/新机 LOAD 的 41,756 个脉冲序列化为固定
SHA-256 `c7503b92224d157bd1ba05fa0b1c100a8ddca6c9ea679ec52a2dc517abcead02`，与 TypeScript oracle 一致。

Cartridge/EasyFlash 验收证据（2026年08月11日）：Rust VICE revision 46176 Ocean CRT 的 4 个
银行哈希、37 次 ROM PC 帧采样、活动计数 `$63` 与屏幕 SHA-256 均和 TypeScript oracle 一致；
官方 EasyProg 1.6.3 在 109 帧进入 BASIC、264 帧内识别 AM29F040B 双芯片与 1 MiB 卡带，并由
真实 6510 torture path 在 Rust 整机中改写 ROML 8,184 字节和 ROMH 1,679 字节。

REU 验收证据（2026年08月11日）：Rust 整机通过固定 SHA-256 的 VICE revision 46176
QuickReu 1.1.1 全部 8 个功能 PRG，均报告零失败类别；每个程序完成 44,588 个 DMA 总线周期，
并覆盖 copy/fetch/swap/verify、autoload、IRQ、`$FF00` 触发、经典 REU 尺寸与镜像、
VIC 优先停顿、隐藏 RAM/页 generation、扩展槽互斥和粗粒度 Wasm 持久化 ABI。

完整 save-state 验收证据（2026年08月11日）：格式 v2 `RC64VM02` 使用长度边界、CRC32 和精确
解码，v1 镜像继续可载入；CPU 微周期、虚拟时钟、64 KiB RAM/映射、VIC/CIA/SID、IEC、1541
与 D64/G64、Datasette/TAP、Cartridge/EasyFlash（含进行中的 flash 命令）及 REU/DMA 均可原子
恢复并确定性继续。C64 firmware 由目标 runtime 注入，不写入状态文件。提交 `8948f68` 通过本地
完整 Rust/TypeScript/Wasm/VICE 门禁及远端 [CI run 31491094157](https://github.com/JamesLinYJ/rust-c64-emulator/actions/runs/31491094157) 的全部 job。

生产 Strict 验收证据（2026年08月11日）：提交 `b6c6fdf` 将真实 firmware 注入的 Rust/Wasm
整机放入 module Worker，生产 React 页面不再实例化 TypeScript 硬件；键盘/双控制口/RESTORE、
BASIC PRG、完整 VIC 帧和 SID PCM 均通过粗粒度消息边界。帧与 PCM 使用有界可回收 Transferable
双缓冲，TypeScript 核心仅保留为测试 oracle。本地完整 Rust、TypeScript、Wasm、VICE 参考链、
Sites 无 source map 打包及 Chromium 桌面/移动端门禁通过；提交 `d5fcae5` 修正 CI 只测试普通
Vite bundle 的缺口，远端 [CI run 31496827909](https://github.com/JamesLinYJ/rust-c64-emulator/actions/runs/31496827909)
确认 Node 22/24、Rust/Wasm、Sites 生产产物与 Chromium smoke 全部通过。

## M2：Turbo 语义

- [x] 实现每系统周期 2..64 个内部 CPU 槽位和一次性锁定 Auto 档位。
- [x] 实现物理页能力、代码版本、映射 generation 和统一地址空间分类。
- [ ] 实现强顺序 BusBridge、VIC 优先、REU/DMA 所有权和精确事件退出。
- [ ] 实现自修改代码、隐藏 RAM、Cartridge bank 和 DMA 写入失效。
- [ ] 实现 opt-in `$D030/$D031`、`$D07A/$D07B`，Reset 恢复 Strict/1MHz。

验收：Strict/Turbo 随机差分和全部桥接边界通过，不存在软件专用路径。

阶段进展（2026年08月12日）：简单指令执行器已在 2、4、8、12、16、20、24、32、48、64
档位上连续执行 4,096 个纯 RAM CPU 槽位，并与 Strict 的 CPU 状态、RAM 写入和总线事务逐项一致；
虚拟时钟现能给出不得越过的下一外设事件边界。`CodePageGuard` 同时校验物理页能力、页代码
generation 和全局映射 generation，CPU、REU、Enhanced DMA、Host 写入共用同一失效路径。
Turbo CPU 与 REU DMA 访问已通过强顺序 `BusBridge`：处理器端口映射变化、REU 即时 DMA、
`$FF00` 延迟触发及 REU 写入代码页失效均有根因测试，IO2 被 PLA 屏蔽时也不会误判为 REU。
Strict CPU 继续走无额外桥接 trace 的原周期精确热路径。VIC/Enhanced DMA 主控的统一桥接、block
执行器精确退出和本节其余验收尚未完成，因此 M2 不标记完成。

## M3：Turbo 执行引擎

- [ ] 建立预解码 basic block 和紧凑 uop IR。
- [ ] 加入页依赖 guard、热后继、分支目标缓存和 hot trace。
- [ ] 加入安全 uop fusion、lazy flags、延迟写回和精确 deopt。
- [ ] 热路径无临时分配；MMIO 永不推测执行；保持 CSP-safe。

验收：固定 runner 上 BASIC ≥10×、纯 RAM ≥15×、通用 I/O 混合负载 ≥3×，输出哈希一致。

## M4：平台运行时

- [x] 将 Wasm VM 放入 module Worker。
- [x] 定义 run/pause/reset/media/input/state/diagnostics 命令协议。
- [x] 实现可回收 Transferable 帧/PCM 缓冲；SAB 与 simd128 仅作可选优化。
- [x] Chrome/Edge、Firefox、WebKit、Node 使用同一 Rust 核心。

验收：PAL 50 Hz、NTSC 60 Hz 稳定，UI 不阻塞，参考机器无音频 underrun。

平台协议与跨浏览器语义验收证据（2026年08月11日）：提交 `c298841` 通过版本化粗粒度请求完成
run/pause/reset、输入、完整 save/load state、diagnostics、Tape、Cartridge/EasyFlash、REU 及 1541
D64/G64 媒体控制，Transferable 所有权和 64 位计数器 high/low 边界均有测试；提交 `9d4f141`
让确切 Sites 生产产物在 Google Chrome、Microsoft Edge、Mozilla Firefox 和 WebKit 中分别完成
module Worker/Wasm 启动、PAL 帧推进、暂停/单帧/恢复及 BASIC PRG 执行，Node 使用同一
`c64-core` 生成物。远端 [CI run 31507556499](https://github.com/JamesLinYJ/rust-c64-emulator/actions/runs/31507556499)
的五个 job 全部成功；Chrome、Edge、WebKit 为 50 host FPS，Firefox 为 12 host FPS。因此本项只
关闭“同一 Rust 核心”语义兼容。

PAL/NTSC 与音频本地验收增补（2026年08月12日）：提交 `a18e9d7` 将 MOS 6567R8 的
65×263 周期表、247 行可见帧、整数时钟元数据和动态 Worker 节拍接入 `C64Chipset` 与生产页面；
同一 Sites release 产物在 Chromium 达到 PAL 50 FPS、NTSC 60 FPS，二者均为 0/120 超预算且
AudioWorklet 欠载/溢出计数为零。Firefox 功能路径从 12 提升到 18 host FPS，但仍未达到实时；
远端跨浏览器 CI 通过前，本增补也不能视为最终验收。因此 M4 总验收仍缺 Firefox 实时性能，
不得据此宣称平台运行时整体完成。

Firefox 实时与 M4 最终验收增补（2026年08月12日）：提交 `fe5d56e` 在不改变公开硬件语义的
前提下，为 CPU 被动读、VIC 稳态、
CIA 处理器时钟/TOD 与 SID 静音周期增加精确批处理和派生状态校验；生产 Worker 改用
`MessageChannel` 投递帧任务，消除嵌套 `setTimeout` 的浏览器定时器钳制。相同的无 source-map
Sites 产物在 Google Chrome、Microsoft Edge 和 Mozilla Firefox 均达到 50 host FPS；本次 Firefox
p95 为 20.00 ms、120 帧中 9 帧超预算，并完成 BASIC PRG 路径。本地 `npm run check`、
`npm run verify:wasm`、固定版本完整参考链和 326,815-byte Wasm 打包均通过。Fedora 45 本机缺少
Playwright WebKit 固定依赖的 ICU 74/libjpeg 8，因此本机未运行 WebKit；远端
[CI run 31539712745](https://github.com/JamesLinYJ/rust-c64-emulator/actions/runs/31539712745)
已让 Chrome、Edge、Firefox、WebKit 分别以 50 host FPS、p95 3.20/2.30/16.00/7.00 ms 完成同一
生产 Worker/Wasm 与 BASIC PRG 路径，Node 22/24、Rust/Wasm、Chromium UI 和跨浏览器五个 job
全部成功。M4 平台运行时验收至此关闭。

## M5：SuperCPU

- [ ] 实现 W65C816 emulation/native、24 位地址、M/X、寄存器、向量和周期。
- [ ] 实现 decimal、WAI/STP、block move 和 bank 0 C64 映射。
- [ ] 提供最多 16 MiB 直接 Fast RAM、公开控制寄存器和 20 MHz preset。
- [ ] 仅加载用户提供且经过哈希校验的 firmware。

验收：独立 65C816 指令向量、随机差分和 SuperCPU 兼容语料通过；Reset 为 E=1、1MHz。

## M6：VM Enhanced

- [ ] 定义版本化 capability discovery，不增加私有 CPU opcode。
- [ ] 提供最多 256 MiB 按需分配的额外内存池。
- [ ] 实现 32 位 copy/fill/scatter-gather/chained DMA、IRQ、错误与重叠语义。
- [ ] CPU、VIC、REU、Enhanced DMA 共用页一致性协议。

验收：DMA 属性/模糊测试覆盖越界、链、重叠、IRQ、失效和 save-state replay。

## M7：Stable

- [ ] 固定公开兼容语料、来源、版本和 SHA-256。
- [ ] 运行 Rust native/Wasm、TypeScript oracle、浏览器和 Node 矩阵。
- [ ] 记录语义速度、宿主帧预算、underrun、block hit、失效和 deopt 原因。
- [ ] 完成架构、模式、状态兼容、firmware 政策和可复现构建文档。
- [ ] Strict、Turbo、SuperCPU、Enhanced 独立达到门槛后分别标记稳定。

## 每次变更门禁

1. 先写能够复现缺口的确定性测试。
2. 实现公开架构规则，不添加程序或 benchmark 特判。
3. 运行 `cargo fmt --check`、Clippy `-D warnings`、Cargo tests、Wasm tests，输出不得包含 warning。
4. 运行 `npm run check` 和仓库全部 VICE 驱动的 `npm run verify:reference`；涉及平台时运行 `npm run verify:browser`。
5. 检查 diff，只包含本阶段相关文件，且未改动无关日期。
6. 在用户要求发布后才 commit、push；远端 CI 通过后再关闭对应事项。
