# Rust C64 Emulator 体系结构规范

版本：1

创建日期：2026年08月10日

维护者：OpenAI Codex

## 兼容契约

机器 profile、执行模式和宿主 pacing 是三个正交维度：

- profile：`stock`、`supercpu`、`vm-enhanced`
- execution：`strict` 或具有明确槽位档位的 `turbo`
- pacing：`realtime` 或 `unbounded`

Strict 是兼容性真值，默认等同 stock PAL C64。Turbo 不改变 opcode、寄存器、状态位、指令周期成本或外设时钟，只让一个 C64 系统周期容纳多个内部 CPU 周期。`unbounded` 只改变宿主等待策略，不改变客机时间。

Reset 保留已安装 profile 和扩展内存，但将 execution 恢复到 Strict/1MHz。SuperCPU/65C816 Reset 同时进入 emulation mode。profile 变化只能通过受控 power-cycle；execution 变化只在提交/系统周期边界生效。

## 虚拟时间与事件顺序

时间戳是整数二元组 `(system_cycle, slot)`。Strict 每周期一个槽位；Turbo 每周期 2..64 个槽位。每个 legacy 周期遵循固定顺序：

1. 锁存到期的外部输入、IRQ/NMI 和 DMA 请求。
2. 执行 VIC φ1 取数并确定 BA/AEC/φ2 所有权。
3. 在合法 φ2 位置完成至多一个等待中的外部总线事务。
4. 执行不越过下一个外部事件的内部 CPU 槽位。
5. 推进 CIA、SID、Datasette、Cartridge、IEC/Drive 等该系统周期状态。
6. 提交可观察输出并进入下一 `system_cycle`。

Strict 的设备微相位继续服从原逐周期模型；上述顺序是 Turbo 域与 legacy 域之间的公开屏障顺序。任何实现优化都必须在这些屏障处得到相同状态。

## 地址空间与 BusBridge

基础 64 KiB RAM 只有一个 backing store。VIC、CPU、REU 和 Enhanced DMA 在各自规定时间读取或写入同一数据，不建立需要软件刷新的影子显存。

每个物理页拥有：

- 当前物理目标和 Fast/BusBridge 域属性
- 代码 generation
- 所依赖的地址映射 generation
- 是否存在读副作用

`$0000/$0001`、可见 I/O、颜色 RAM、Cartridge、REU/DMA 控制和副作用读取始终通过 BusBridge。ROM 与当前映射到基础 RAM 的页可以进入 Fast 域。对源码页的 CPU、REU、DMA 或 host-loader 写入使用同一 generation 失效规则。

Turbo block 在 MMIO、处理器端口、映射变化、自修改代码、IRQ/NMI、VIC/REU/DMA 请求、Cartridge bank 变化或下一个外部事件前精确退出。MMIO 不推测、不合并、不重排。

## Rust/Wasm 边界

`c64-core` 持有全部架构状态和虚拟时间；`c64-wasm` 只负责粗粒度命令与数据搬运。JavaScript 不逐 CPU/VIC 周期回调 Rust。浏览器和 Node 从同一 crate 构建，Wasm memory growth 后由 facade 更新 typed-array view generation。

生产 ABI 最终包含：创建/销毁、运行到时间点、pause/reset、execution 请求、媒体、输入事件、save/load state、帧/PCM buffer 和 diagnostics。SIMD、SharedArrayBuffer 与宿主线程是可选、bit-identical 的传输/批处理优化。

## Save-state v1

状态使用 little-endian section 格式：8 字节 magic、16 位版本、16 位 section 数量，随后为四字节 tag、32 位长度和 payload。v1 基础 section 为 `CONF`、`TIME`、`CPU0`、`RAM0`。未知 section 可跳过；缺失必需 section、长度错误或不支持版本必须显式失败。

翻译 block、hot trace、宿主指针、音视频 staging buffer 和性能计数不是架构状态，不写入 save-state。载入后统一失效并重建派生缓存。

## 禁止的软件专用行为

运行时不得以程序名、ROM 内容、磁盘/磁带哈希、屏幕文字、已知 PC 序列或 benchmark 身份选择执行路径。允许使用哈希的地方仅限外部资源完整性和测试 fixture 固定版本，不能影响客机语义。
