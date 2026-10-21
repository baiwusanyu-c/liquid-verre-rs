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
> 它们依赖 DDA 桌面复制、真实 vsync 节拍与可交互的桌面会话（`SetCursorPos` 物理停放光标、
> `GLASS_INTERNAL_SHOT` 落盘取证图、`[metrics]` 行里的 `rel_max` / `acq_max` 健康判据），
> GitHub runner 的无交互会话里既采不到桌面也测不出有意义的 fps / 延迟 —— 跑出来只会是假绿或假红。
> **替代口径**：这两类阀门只能在物理 Windows 桌面上按本节的健康判据跑，结论抄进
> `performance_report.md`（阈值变更另见 §6）。CI 绿只代表「编译 / 单测 / 静态检查 / crate 可打包」，
> **不代表视觉与性能没回归**；合并前的视觉与性能结论仍以本机取证为准。

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

## 4. 阀门与阈值（`tools/gate_perf.py`）

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