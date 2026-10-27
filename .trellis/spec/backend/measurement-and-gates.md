# 测量方法论与质量阀门

## 0. 被测对象与命令（重构后的 workspace 口径）

被测可执行文件是 **dev playground 产物**，不是库本身（库没有 bin）：

```
target/release/liquid-verre-playground.exe
```

`tools/*.py` 里全部脚本都用同一套覆盖规则：

| 环境变量 | 覆盖谁 | 默认值 |
|---|---|---|
| `VERRE_EXE` | 当前被测二进制 | `<repo>/target/release/liquid-verre-playground.exe`（`cmp_visual.py` 例外，默认 `<repo>/tools/exe_now.exe`） |
| `VERRE_OLD_EXE` | 跨版本对比用的旧二进制 | 只有 `cmp_visual.py` 用，默认 `<repo>/tools/exe_old.exe` |

另：`tools/verify_all.py` 的第 1 项（跨版本 overlay/composed 逐位对比）还硬编码了
`BASE_EXE = D:\github-project\lg-base\target\release\liquid-glass.exe` —— 那是把重构前提交
`82a49e8` 拉成独立 worktree 后构建的产物，需要自己先建好（步骤见 `performance_report.md` 第七节）。
没有它时该项会因缺文件而 FAIL，属预期。

**库默认不自建任何玻璃窗口**（那是上层应用的职责）。取证脚本需要一块确定性的默认玻璃，
因此每个脚本都显式设 `GLASS_DEFAULT_BLOCK=1`（等价于 `EngineConfig.diagnostics.default_window = true`，
playground 走后者）。默认块的尺寸与重构前一致：
`w = clamp(主屏宽 * 0.62, 720, 1200)`、`h = clamp(主屏高 * 0.68, 480, 760)`、居中。
**不设这个开关就跑取证 = 空场景**，脚本会得出「0 差异」这种没有判别力的结论。

### 阀门命令（两条路径等价，改一边就要改另一边）

```powershell
# 路径 A：cargo xtask（.cargo/config.toml 里 alias 到 run --package xtask --release --）
cargo xtask build                 # = cargo build --workspace --release
cargo xtask check                 # = fmt --all --check + clippy --workspace --all-targets --release -D warnings
                                  #   + test --workspace --release（三步全跑完再汇总）
cargo xtask gate                  # = 先 build，再 python tools/gate_perf.py（透传退出码）
cargo xtask package [--out DIR] [--no-zip]   # 收集产物到 dist/ + build-info.json + zip
cargo xtask clean                 # = cargo clean + 删 dist/

# 路径 B：Node 脚本（零 npm 依赖，跨平台，固定 UTF-8）
node scripts/build.mjs [--debug] [--only <pkg>]
node scripts/check.mjs
node scripts/gate.mjs [传给 gate_perf.py 的参数]
node scripts/package.mjs [--out DIR] [--no-zip] [--debug]
node scripts/zip.mjs <sourceDir> <outZip>
```

单独跑某一层：

```powershell
cargo test --release --workspace                                    # 功能阀门
cargo clippy --workspace --all-targets --release -- -D warnings     # 静态检查
cargo fmt --all -- --check
python tools/gate_perf.py                                           # 性能阀门
cargo run --release --bin liquid-verre-playground -- --port 8787    # 手动看效果
```

`xtask` 与 `scripts/` 只做**编排**，不存阈值：性能阈值只在 `tools/gate_perf.py`，
功能判据只在 `cargo test`。

## 1. 机器健康判据（不满足则数据作废，不得下结论）

`rel_max < 5 ms`、`acq_max < 25 ms`、`dda_acq ≈ 桌面实际重绘率`
（三个字段都在 `[metrics]` 行里：`rel_max_ms` / `acq_max_ms` / `dda_acq`）。
劣化签名：`rel` 200~900 ms、fps 2~40、日志出现「独占全屏应用」。
处置：等 / 关掉占用 GPU 的进程 / 注销重登，**不要**在劣化窗口里做 A/B 或下性能结论。

> 并行开发时特别注意：另一个 agent 在跑 `cargo build` / `cargo test` 就会把 CPU 吃满，
> 直接把测量窗口打成劣化窗口。**性能取证期间不要同时构建。**

> **CI 上跑不了本文的任何测量**（这是刻意的，不是遗漏）：`.github/workflows/ci.yml` 只覆盖
> `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets --release -- -D warnings`
> + `cargo test --release --workspace` + `cargo package -p liquid-verre-rs --list` 干跑；
> `release.yml` 也只多跑 `node scripts/check.mjs` + `node scripts/package.mjs`。
> **像素取证（`cmp_visual` / `verify_*` / `repro_tremble`）与性能阀门（`gate_perf`）一律不进 CI**：
> （0.5.0 起 `verify_*` 含功能行为取证三件套：`verify_input_sink` / `verify_content` /
> `verify_focus_drop`——触碰输入路由/内容层/焦点路径必复跑；三件均按「判别力」设计，
> 断言禁止"无证据也 PASS"的逃生口。）
> 它们依赖 DDA 桌面复制、真实 vsync 节拍与可交互的桌面会话（`SetCursorPos` 物理停放光标、
> `GLASS_INTERNAL_SHOT` 落盘取证图、`[metrics]` 行里的 `rel_max` / `acq_max` 健康判据），
> GitHub runner 的无交互会话里既采不到桌面也测不出有意义的 fps / 延迟 —— 跑出来只会是假绿或假红。
> **替代口径**：这两类阀门只能在物理 Windows 桌面上按本节的健康判据跑，结论抄进
> `performance_report.md`（阈值变更另见 §6）。CI 绿只代表「编译 / 单测 / 静态检查 / crate 可打包」，
> **不代表视觉与性能没回归**；合并前的视觉与性能结论仍以本机取证为准。

### 1.1 取证必须对「用户前台开着哪个应用」保持 hermetic（`GLASS_FS_HIDE=0`）

`platform::display::fullscreen_app_on` 的形状判据是「前台窗口 = `WS_POPUP` 且无
caption/thickframe 且铺满整块屏」，而**无边框最大化的普通应用**（Electron / Tauri 那一类：
ChatGPT 桌面版、VS Code、app-mode 浏览器）完全符合这个形状。命中后画布被隐藏，
且（0.4.2 起）**整屏停产** —— 于是取证脚本会在「前台恰好是这类应用」时拿到 0 帧连拍
并整批假 FAIL（实测 `repro_tremble far/many` 只有 0~1 帧、`verify_adaptive` 报「未观察到降档」）。

因此 `tools/` 下所有像素 / 性能脚本都必须设 **`GLASS_FS_HIDE=0`**（已注入 13 个脚本），
唯一例外是 `tools/verify_occlusion.py` —— 它就是测这条停产门的，刻意不设。

⚠ 同类陷阱：**帧数 / fps 偏低不一定是代码问题**。同一配置内 `repro_tremble` 的连拍帧数
能从 4 帧跳到 17 帧（采集后端处在 DDA `DuplicateOutput 拒绝访问 0x80070005` → 回退 WGC 的
劣化窗口时，`cap` 段单圈阻塞 124 ms、`dda` 只有 2.6/s）。归因必须用
「把开关关掉 ≈ 改动前行为」做**配对 A/B**（`GLASS_OCCLUSION=0` + `GLASS_FS_HIDE=0`），
不能看单次数字下结论。

⚠ 取证脚本自己的窗口也是负载源：一个铺满屏的探针窗口若**整窗** 10 Hz 重画，
等于每秒 6500 万像素的 GDI 填充，足以把机器打成劣化窗口（实测 `loop_hz` 掉到 2、
`max_loop` 913 ms）。只需要「桌面有脏区」时，闪一小块就够（`verify_occlusion.py`
闪屏幕正中 96x96、两色只差 1 LSB）。

### 1.2 劣化窗口里哪些判据仍然有效（不要一刀切地全部重跑）

§1 的 `rel_max < 5 ms` / `acq_max < 25 ms` 作废的是**阈值型**数字。判据要分两类看：

| 类别 | 例子 | 劣化窗口下 |
|---|---|---|
| **等价关系型**（同一进程内两帧逐位相等、计数器必须为 0） | `verify_dirty`（增量 == 全量）、`verify_validity`（虚报/漏画计数）、`verify_mouse_isolation`（闭包外逐位一致）、`cmp_visual` 的开关 A/B | **仍然有效** —— 机器慢只减少帧数，不会让「两帧相等」变成「不相等」；反而要注意别把「样本太少」误读成「通过」 |
| **帧数依赖型**（需要足够多的连拍样本） | `repro_tremble`（连拍帧数有下限） | **看帧数**：帧数达标则结论有效，未达标是「无法分析」而不是 FAIL |
| **阈值型**（fps / p50 / CPU / Private 对标固定门檻） | `gate_perf`、`ab_latency`、`measure_*` | **一律作废**，不得下任何结论 |

所以劣化窗口里正确的动作是：**继续跑等价关系型，停止重试阈值型**，并且把「为什么这一项没取到
有效数据」连同**配对 A/B 的排除证据**一起记下来（形式：把特性全关 ≈ 改动前行为，
若数字同样烂，即证明瓶颈在机器不在代码 —— 实测 `own_us` 73 vs 78 µs、
15.9 vs 18.8 fps，关掉代码机器照样 15.9 fps）。

⚠ **顺序纪律**：阈值型取证**独占机器**。`cargo build` / `cargo test` / 子代理编译
都会把窗口打成劣化（实测反复出现），所以「先跑完 `gate_perf`，再跑任何会触发编译的东西
（包括 `trellis-check`）」。重启机器后第一件事就是 `gate_perf`，此时 `target/` 里的产物还在，
不需要重新构建。

### 1.3 归因之前先怀疑测量装置本身（本项目的真实前科）

同一批"机器单调恶化"的观测（`repro_tremble` 帧数 22→2、A/B 两臂散布 19~130%、
`Progman` 间歇命中独占全屏判据），最后查出的真因是**自己的取证脚手架**：
`tools/` 里 12 个脚本启动 playground 时不带 `--no-browser`，而
`playground/src/panel.rs:68` 会 `cmd /c start` 开默认浏览器，
playground 被 terminate 后**标签页不关** => 每次采样累积一个页面。

⇒ 三条规矩：
1. **启动被测进程的脚本一律带 `--no-browser`**（已全量修好并在 `Popen` 前留注释）。
   新写脚本时照抄 `perf_probe.py` 的写法。
2. 出现"数字越跑越差"时，先回答"**我在被测进程之外还起了什么、它退干净了吗**"，
   再谈机器状态。（我当时归因给"开机后后台任务爬升"，错了十几轮，还顺手重编了一个参考
   二进制来"证明"非回归 —— 那套两档判定本身是对的，但结论建立在错误的归因上。）
3. 两档判定的价值正在这里：**绝对档在一台不干净的机器上会连喂 4 个假 FAIL**，
   相对档因为两臂同等受灾而仍能给出正确答案。

## 2. 统计规则

- 一律**轮转交错 + ≥3 重复 + 中位数/p95**；均值在 10~100 loops/窗口 下无意义。
- 内存只看稳态窗口中位数；单次运行的 max 会被瞬态污染。
- 对比必须同环境相邻进行（A/B 二进制轮换），跨小时对比无效。
- `gate_perf.py` 内部已经是「每个场景一次 `perf_probe`（10 s + 3 s warmup），
  对窗口内的多条 `[metrics]` 行取中位数」。

## 3. 确定性工具与「哪个 oracle 验什么」

| 工具 / 开关 | 验什么 | 注意 |
|---|---|---|
| `GLASS_SYNTH_CAP=1` | 跨进程 / 跨时间逐像素可比 | 合成源只由像素坐标决定；`SIM_BACKDROP` 只伪造脏矩形，不改内容 |
| `GLASS_DEFAULT_BLOCK=1` | 给取证一块确定性的默认玻璃 | **库默认不建窗口**，不设就是空场景 |
| `GLASS_CURSOR_AT=x,y` | 钉死光标 | 重构后**全程生效**（`App::pinned_cursor` → `read_cursor()`）；重构前只设初值，主循环立刻回到真实光标 |
| `GLASS_FAR_GRID=1` | 4 块互不重叠的分离布局，跨块隔离取证 | 不能用 `GLASS_MANY` 代替：20 块密集重叠，邻近余晖沿半透明阴影逐块传递，判定集合的传递闭包会吞掉全部 20 块 |
| `GLASS_INTERNAL_SHOT_AT_MS=<ms>` | 拍**稳态帧**而不是引导帧 | 必须 > 环境色 EMA 收敛时间（τ=150 ms）；`cmp_visual.py` 用 2500 |
| `GLASS_SHOT_EVERY_MS=<n>` | **时间维度**稳定（颤抖 / 闪动） | 连拍**不**强制全量重画（保留增量路径，那才是用户看到的路径）；默认只写 composed，`GLASS_BURST_OVERLAY=1` 才附带 overlay/capture/soft/frost |
| `GLASS_SIM_BACKDROP=WxH@HZ` | 确定性触发节拍（吞吐测量） | **不能**用于「增量==全量」像素 oracle |
| `GLASS_SIM_BUSY=<ms>` | 驱动自适应控制器闭环 | 只在拖拽期注入 |
| `GLASS_VERIFY_VALIDITY=1` | 金字塔虚报 / 漏画计数 | 计数器 oracle，最稳 |
| `GLASS_ADAPTIVE=0` | 关掉控制器做「基准视觉」比对 | `cmp_visual.py` 默认关，避免降档改视觉 |
| `verify_dirty` | 脏区书本账（增量==全量） | 真实桌面；取证帧强制全量，故测不出持久化类缺陷 |
| `repro_tremble far\|many` | 拖一块时其他块逐帧不变 | 静态合成源；排除首 2 帧与末 1 帧（teardown） |
| `cmp_visual` | 跨版本视觉等价（见 §3.1） | 需要 `VERRE_OLD_EXE` / `VERRE_EXE` 两个二进制 |
| `verify_monitors_rebuild` | 显示器变化重建路径行为（#1/#4/#5） | `PostMessage(WM_DISPLAYCHANGE)` 广播刺激，不碰系统显示配置；判别力=基线实测 3 红；断言含「块 rect 逐位不变」（scale≠1 屏才有牙齿，脚本自报判别力） |
| `verify_clickthrough_recovery` | 遮挡停产后穿透自锁解除（#3） | 盖块→停产日志→移开→进块 ≤300ms 摘穿透 + 块外反向对照 + 物理拖拽终局；无停产日志=FAIL（禁止空转 PASS） |

**教训（勿重犯）**：按类名枚举画布**必须过滤 pid**（`GetWindowThreadProcessId`）——跨进程残留与孤儿窗口（0.5.1 #5 前科：`GLASS_MONITORS` truncate 泄漏的恒穿透孤儿）会混进 `FindWindowExW` 枚举，刺激打中孤儿 = 静默蒸发，把活测成死（rebuild 脚本第一版栽过）。

**教训（勿重犯）**：`SIM_BACKDROP` 场景下做「增量==全量」像素对比是无效 oracle
（假脏区外的真实桌面变化是合理陈旧；合成源下脏区恒 `[0,0,0,0]` 使金字塔永不跟踪）——
该场景只能用计数器 oracle。

### 3.1 `cmp_visual.py` 的新口径（重构后）

三个问题：

1. **跨版本稳态帧**：旧二进制 vs 当前二进制，同一份确定性合成采集源、同一光标位置，
   按 **deep / interior / ring / outside 四段**统计。
   分段常量：`RING = 24`（边框斜面环的粗略宽度）、`DEEP = 130`
   （`bevelWidth ≈ lerp(26, min(hs), 0.15) ≈ 79 px`，`edgeWeight` 支撑 `≈ bevelWidth*1.35 ≈ 107 px`，取 130 留余量）。
   拍的是**稳态帧**：`GLASS_INTERNAL_SHOT_AT_MS = STEADY_MS = 2500`。
   - **single 布局：四段必须全 0 差异**（这是「视觉效果不变」的硬证明）。
   - **many 布局：只作参考**——圆角语义是**有意变更**（见下），因此判据改成两条：
     (a) `now vs now` 每段**最大通道差 <= 2 LSB**（不要求硬 0）：frost ping-pong 与
         1/16 层都是「按脏区部分刷新」，跨运行的刷新序列在桌面繁忙时会差 ±1 LSB ——
         这是重构前就有的管线特性（旧二进制同样如此），同一次运行内「增量 == 全量」
         由 `verify_dirty.py` 严格证明。single 布局 footprint 小、首帧即全覆盖，
         因此 single 的 `now vs now` 仍然硬 0；
     (b) `old vs now` 的每一处差异都必须能归因到圆角：
     以每块四个圆角圆心（偏距 = `R_NEW = 32.0`）为心、
     `R_NEW + R_TOL = 38 px` 为半径的邻域**之外**，最大通道差必须
     `<= FAR_MAX_DIFF = 2`（实测残差 ≤ 2 LSB / 3105 像素，来自半径变化经 SDF
     对高斯阴影尾部的轻微影响）。超过这个值就说明有别的东西变了。
2. **光标移动（当前构建）**：光标在玻璃「深内部」两点之间移动（`DEEP_A` / `DEEP_B`，
   都距边 ≥ `DEEP + 60`），**deep 段必须 0 差异** —— 证明鼠标移动不会让玻璃里的**背景**颤抖。
   斜面环（bevel）里的高光 / 描边响应是设计意图，不计入判据。
3. **真实桌面**：各拍一张 composed 图供肉眼看过曝 / 对比度，不做逐像素比较。

**光标如何固定（两条一起用）**：

1. `GLASS_CURSOR_AT` 钉死（重构后全程生效；旧二进制只拿它做初值，因此对旧二进制无效）；
2. `SetCursorPos` **物理停放** + `CursorWatchdog` 线程每 0.12 s 复位一次
   （对旧二进制唯一有效的办法；会短暂抢走鼠标控制权，只在取证期间开启，退出立刻停）。

**时间依赖项必须冻掉**：合成源（`GLASS_SYNTH_CAP`）只替换采集纹理，`changed` 仍来自
真实桌面的 DDA 元数据；桌面一活跃，环境色 EMA 就按墙钟 dt 推进（`alpha = 1-exp(-dt/tau)`），
两次跑的推进时刻不同 ⇒ 全图 ±1~2 LSB 漂移（实测 single deep 段 449582 像素 / 最大 2，
且 now-vs-now 也不再自洽）。因此 cmp_visual 一律设 `GLASS_ENV_EMA=0`
（`env_rt()` 退回 1/16 层，确定性）；EMA 自身的等价性由 `verify_env_ema.py` 在**首帧**
单独证明（首帧 alpha 强制 1，开/关逐位一致）。

停放点 `SAFE_CURSOR = (1560, 1900)`：主屏下方空白区，离单块玻璃与 20 块密集布局
**都 > 3 × mouse_radius**（默认 220 ⇒ 660 px），于是鼠标点光的有限支撑窗 `gate` 精确为 0，
两个二进制都不会画任何 hover 相关效果 —— 结果与「跑测试时鼠标碰巧停在哪」无关。
这一点是实测踩出来的：**停在块内时**，重构后新增的边缘 / 对角缩放 border 会让整片圆角对不上
（最大通道差 140）。

**为什么比稳态帧而不是首帧**：重构前 `src/ui/panel.rs` 在主循环里每帧把参数写回 `app.params`，
而它用 `Arc<AtomicIsize>` 存值，于是 `tint_opacity` 0.018 被量化截断成 0.01、
`blur_min` 被面板派生成 `blur_max * 0.12 = 2.4`。旧二进制的**引导帧**（面板 apply 之前）
与**稳态帧**（之后）参数不同，实测两者相差 873748 像素、最大通道差 140。
用户长期看到的是稳态帧，所以取证以稳态帧为准。库化之后不再有量化往返，两帧同一份参数。

**many 布局的圆角差异是有意的行为变更**：重构前主循环里
`if target_id >= 0 { 改选中块 } else { 改全部块 }`，而 `sync_selection` 总会把 `selected_id`
兜底成**最后一块**，于是 20 块布局里有 19 块的 `radius` 永远停在 `Block::new` 硬编码的 `28.0`。
重构后半径统一由样式派生（`Block::effective_radius(style) = clamp(style.corner_radius() * scale, 0, 短边/2)`），
默认 `corner_radius = 32.0` ⇒ 20 块全部 32。这是需求 F1「全部参数配置化」的直接结果，
**不是回归**，但跨版本比对 many 布局时圆角带必然不同。

## 4. 阀门与阈值（`tools/gate_perf.py`）—— 两档判定

### 4.1 为什么改成两档

绝对阈值（`many-drag >= 60` 等）在这台机器上**没有余量**：0.4.1 基线记录 60.9，
而机器状态一漂移就落到 59.2；实测**关掉被测特性后逐位相同 59.2** ⇒ 那 0.8 不是回归，
是尺子贴到了机器能力线。把绝对值调低来放行等于**静默削弱闸门**（下次真回归就不响了）。
正解是「和同一台机器上的上一个版本比」。

### 4.2 档位

| 档 | 触发条件 | 硬判据 | 绝对值 |
|---|---|---|---|
| **相对 A/B** | 设了 `VERRE_OLD_EXE` 且文件存在 | 每个场景 `REPS` 轮 x 2 臂（**奇偶轮反序**抵消漂移）取中位数，新臂 fps 劣化 > `GATE_REL_MAX`（默认 3%）判 FAIL；`Private >= 200 MB` 恒为硬判据 | 只打印 + `ABS-DRIFT` 提示"该重新标定"，**不决定结论** |
| **绝对（回退）** | 没设 / 文件不存在 | 沿用 4.3 的绝对门槛表；`Private` 同样硬判 | 就是判据本身 |

**噪声优先于结论**：`spread_pct`（逐轮 fps 的 max-min/中位数）取两臂较大者，
若 `散布 >= 劣化 x 0.8` ⇒ 判 **「结果不可信」并 FAIL**，措辞明确要求"换健康窗口重跑或加轮次"，
**既不许宣布回归、也不许签字放行**。没有这条，3% 门槛在噪声 17% 的窗口里等于抛硬币
（实测被"新臂莫名快 17.6%"骗过一次，也是 `REPS` 默认从 2 提到 **3** 的原因 ——
2 轮不是省时间，是判据失效）。

`tools/test_gate_logic.py`（纯逻辑、秒级、不启动 EXE）钉住 19 条判定路径：
两档各自的通过/抓出、"判回归"与"判不可信"互斥、新臂更快不判失败、
`Private` 在两档下都硬判、`REPS>=3`、`spread_pct` 的除零/单样本边界。
**改 `gate_perf.py` 的判定必须同步跑它。**

### 4.3 绝对门槛表（回退档；也是相对档的"该重新标定"参照）

四个场景：`single-drag`（lat 默认 1）、`many-drag`（lat 默认 1）、
`many-drag` + `GLASS_FRAME_LATENCY=2` + `GLASS_SIM_BACKDROP=900x700@120`、
`transparent-drag`（透明玻璃单块，`perf_probe` 以 `--transparent --no-demo-pair`
启动 playground，几何与 `single-drag` 完全可比）。

**硬门槛（FAIL ⇒ 退出码 1）**：

| 判据 | 阈值 |
|---|---|
| 单块拖拽 | `fps ≥ 90` **且** `p50_loop ≤ 9.0 ms`（120 Hz 屏必须锁相） |
| 20 块默认档 | `fps ≥ 60` |
| 20 块吞吐档（lat=2 + 合成 120 Hz 负载） | `fps ≥ 90` |
| 透明模式单块拖拽（需求 F3.1 / 验收 A9） | 与液态单块**同一套**：`fps ≥ 90` 且 `p50 ≤ 9.0 ms` |
| 常驻内存 | 四个场景的 `idle Private` 全部 `< 200 MB` |

**软门槛（只 WARN）**：20 块默认档 `fps ≥ 90`；拖拽 CPU `< 5%` 单核
（两者都受真实桌面重绘率 / DWM 背压支配，不是渲染成本）；
透明模式 `own_us` 高于液态单块 +10µs 以上（跳过整条 Pass A 后本应更低，
若更高说明 `needs_backdrop` 汇总 / `FrameInput` / compositor 跳过链路断了）。

每次运行都输出 `dda_acq` / `dirty_mpx_avg` / `own_us`（= `stage_us[0]+[1]+[3]` 的中位数），
用于区分「我们慢」与「桌面 / DWM 忙」。

### 4.3.1 重启后的可用窗口只有 ~10-15 分钟（实测时间线，2026-10-22）

同一天两次重启后的形状完全一致，务必据此安排跑测顺序，别把窗口浪费在准备动作上：

| 重启后 T+ | 实测 | 可用性 |
|---|---|---|
| ~5 分钟 | `dda 115/s`、`fps 115`、`p50 8.3 ms`、`rel_max 6~8 ms` | **可用** |
| ~15 分钟 | 24 次采样的 A/B 跑完，臂内散布已涨到 43~88% | 勉强，部分场景判"不可信" |
| ~25 分钟 | 两臂一起塌到 `fps 6~22`、`dda 10~33/s`、`p50 90~180 ms` | **彻底不可用** |

⇒ **`gate_perf` 是重启后第一件事**：不要先跑健康探测、不要先跑单测/自测、不要先 `cargo build`
（`SECONDS_PER_CASE`/`GATE_SECONDS` 见下）。健康判断可以直接读 A/B 自己的臂内散布 ——
它比单独一次探测更诚实。
⇒ 想稳过窗口就 `GATE_SECONDS=6`（24 次采样压到 ~4.5 分钟）；判据严格度不变，
样本变少导致的噪声上升会被 `spread_pct` 自动挡成"不可信"。

### 4.3.2 `many-drag >= 60` 的重标（2026-10-23，干净机器实测）

**旧判据在这台 120 Hz 机器上物理不可稳定达成**，实测四组数据（浏览器污染修掉之后）：

| 来源 | fps | p50 |
|---|---|---|
| 新构建（绝对档首跑） | 58.0 | 16.66 ms |
| 新构建 | 58.9 | 16.66 ms |
| **0.4.1 参考臂** | **59.3** | 16.66 ms |
| 新构建（重标前复跑） | **61.4** | 16.66 ms |

`p50` **每一次都精确等于 16.66 ms = 2 个 vsync 间隔**（120 Hz 屏，lat=1 默认档）。
⇒ 该场景的物理上限就是 **60.0 fps**，抖动把实测推到 58~61.4 之间 —— 门槛 60 正好压在
量化悬崖上：**过不过取决于抖到哪一侧**（历史上那次 60.9 "通过" 同理）。
0.4.1 参考臂同一台干净机器也只有 59.3 ⇒ 这是**门槛定错**，不是代码慢
（`own_us` 只有 21~36 µs，占 8333 µs 帧预算的 **0.3~0.4%**）。

**重标不是把 fps 线挪低放行了事**，而是换成两条管得住的判据：

| 判据 | 值 | 抓什么 |
|---|---|---|
| `fps >= 55` | 悬崖之下留 5 fps 余量 | 掉到 3 个 vsync（= 40 fps）这一级退化 |
| **`p50 <= 17.5 ms`** | **防回归主力** | 节拍从 16.66 ms 滑到 25 ms（3 个 vsync）——旧判据只看 fps 时，这种退化会被抖动的 fps 完全掩盖 |

`tools/test_gate_logic.py` 用三条用例把这次重标的**两面**都钉住：`59.2 + 正常节拍 => 不再假 FAIL`、
`56.0 + p50 25 ms => 必须 FAIL`（主力那条）、`54.0 => FAIL`。改门槛必须同步改这三条。

⚠ 结论层面记一句：`many-drag` 默认档想真正跑到 ≥90 只能走吞吐档（`GLASS_FRAME_LATENCY=2`，
实测 104~106 fps）。lat=1 默认档的 60 fps 上限是第十四轮"跟手优先"的**有意取舍**，
不要为了刷数字把它默认改成 2。

### 4.4 参考二进制怎么来

```powershell
git worktree add ..\tarui-glass-ref <上一个发布的 tag 或 commit>
cd ..\tarui-glass-ref; cargo build --release -p liquid-verre-playground
# 拷出去后删掉 worktree，别把 1MB 二进制提交进仓库
Copy-Item target\release\liquid-verre-playground.exe ..\gate-ref\liquid-verre-playground-x.y.z.exe
git worktree remove --force ..\tarui-glass-ref
$env:VERRE_OLD_EXE="..\gate-ref\liquid-verre-playground-x.y.z.exe"; python ..\tarui-glass\tools\gate_perf.py
```

## 5. 提交前最小集合

```powershell
cargo test --release --workspace
cargo clippy --workspace --all-targets --release -- -D warnings
python tools/gate_perf.py
python tools/verify_dirty.py
python tools/repro_tremble.py far
python tools/repro_tremble.py many
python tools/verify_validity.py
python tools/verify_mouse_isolation.py
python tools/verify_adaptive.py
python tools/ab_latency.py
python tools/ab_memory.py
```

（`verify_dirty` + `repro_tremble far|many` + `verify_validity` + `verify_mouse_isolation` +
`verify_adaptive` + `ab_latency` + `ab_memory` 就是 BASELINE.md 说的「8 个回归脚本」。）

视觉改动额外：

```powershell
python tools/cmp_visual.py            # 跨版本稳态帧四段（需要 VERRE_OLD_EXE / VERRE_EXE）
python tools/verify_crossscreen.py    # 跨屏拖拽 + DPI 换算
python tools/verify_env_ema.py        # 环境色 EMA 开/关 首帧等价
python tools/verify_all.py            # 上述 5 项一次跑完（含跨版本，需要 BASE_EXE worktree）
```

再加连拍目视（`view_image` 看 composed 裁切）。改 blur kernel / scissor 时
`verify_dirty` **连跑 ≥3 次**（间歇残差只会在多次运行里露头）。

上面这套**全部在本机跑**。CI 只等价覆盖前两行（`cargo test` / `cargo clippy`，另加
`cargo fmt --all -- --check` 与 `cargo package -p liquid-verre-rs --list` 干跑）；
从 `python tools/gate_perf.py` 往下一个都不在 CI 里，原因与替代口径见 §1 末尾。

## 6. 结论如何记录

- 数字进 `performance_report.md`（含被否决的方案与理由）；基线口径进 `BASELINE.md`。
- 阈值变更必须在 commit message 与 `performance_report.md` 里写明理由，
  并说明是哪个 oracle、几次重复、中位数还是 p95。
- 取证产物（`tools/*.bmp|png|log|json`、`tools/exe_*.exe`）不入库（`.gitignore`），
  只有**结论**抄进文档。