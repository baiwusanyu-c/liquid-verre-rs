# 日志约定（`log` facade → stderr）

## 1. 门面与后端

- 库内部**一律用 `log` 宏**（`log::info!` / `warn!` / `error!` / `debug!` / `trace!`），
  不再直接 `eprintln!`。依赖里只有 `log = "0.4"`，**不自带日志后端**。
- 库提供一个**可选**的默认 stderr logger：`install_default_logger()` /
  `install_default_logger_with_level(level)`（`src/logging.rs`）。
  `Engine::new()` 会顺带调用后者，因此「什么都不装也能看到诊断输出」。
  它是幂等的、且**不覆盖**上层已装的 logger（`log::set_logger` 只能成功一次，
  第二次返回 `Err` → 函数返回 `false`）。
- **上层想接管全部日志**：必须在 `Engine::new()` **之前**调用 `log::set_logger(...)`
  （`env_logger` / `tracing-log` / 自己的 UI 日志面板都行）。之后再调
  `install_default_logger()` 会返回 `false` 且不产生任何影响。
- 级别过滤：`EngineConfig::log_level`（`LogLevel::{Error,Warn,Info,Debug,Trace}`，默认 `Info`）
  → `log::set_max_level`。上层已装 logger 时以对方的过滤器为准。
  `LogLevel` 是库自己的枚举（不把 `log::LevelFilter` 泄进公共 API，保持 serde 友好），
  有 `From<LogLevel> for log::LevelFilter`。

## 2. 行格式与前缀（**改格式 = 打断质量阀门**）

默认 logger 的行格式与重构前的命令行版本**逐字节一致**：

```text
[<prefix>] <message>\n
```

一次 `write_all` 整行写出（stderr 加锁），避免多线程交错把 `[metrics]` 行撕裂。

前缀由 `log` 的 **`target`** 决定：

```rust
const RAW_TARGETS: &[&str] = &["boot", "verify", "metrics", "adaptive", "capture", "probe"];
```

- `target` 命中 `RAW_TARGETS` → 前缀**就是 target 名**（不再叠加级别名）：
  `log::info!(target: "metrics", ...)` → `[metrics] ...`。
- 否则前缀 = 级别名小写：`[error]` / `[warn]` / `[info]` / `[debug]` / `[trace]`
  （默认 target 是模块路径）。

`RAW_TARGETS` 里 `capture` / `probe` 目前**没有**使用点位，是预留位：
将来给采集层加专用前缀时直接用 `target: "capture"`，不要再往白名单里加名字。
白名单本身有单测（`raw_targets_cover_metrics_prefix`）钉住 `boot` / `verify` / `metrics` / `adaptive`
必须在里面。

### 前缀语义与**谁在解析它**

| 前缀 | 发出点（`target`） | 语义 | 被谁解析 |
|---|---|---|---|
| `[boot]` | `target: "boot"`（`api/engine.rs::new`、`runtime/mod.rs` 设备/着色器/画布三处） | 启动阶段进度与耗时；`Engine::new` 第一行打 `BUILD_INFO.summary()` | 人工排障（驱动卡死时判断卡在哪一步） |
| `[info]` | 默认 target | 状态跃迁（一次性）：显示器清单、刷新率/电源/自适应开关、采集后端与回退、画布显示/隐藏、独占全屏、`GLASS_*` 生效提示 | `tools/perf_probe.py`（摘要里 grep `[info]`/`[warn]` 各取前 12 行） |
| `[warn]` | 默认 target | 降级 / 回退 / 异常但可继续：DDA 失效、WGC 回退、HDR 强制 dda、DPI 感知被宿主占用、某屏画布/合成器创建失败 | 同上 |
| `[error]` | 默认 target | 单屏渲染失败、引擎异常退出 | 人工 |
| `[verify]` | `target: "verify"`（`gfx/compositor.rs` 漏更新自检、`runtime/mod.rs` 全量对照帧） | 取证 / 自检 | **`tools/verify_validity.py`**（判据：虚报与漏画计数必须为 0） |
| `[metrics]` | `target: "metrics"`（`diag/metrics.rs::report_metrics`） | `GLASS_METRICS=1` 的 5 秒墙钟窗口统计行 | **`tools/perf_probe.py` → `tools/gate_perf.py`**（性能硬门槛全部来自这一行） |
| `[adaptive]` | `target: "adaptive"`（`core/quality.rs` 档位跃迁） | 质量档位降级 / 回升 + 丢 vblank 比例 + 该档动作 | **`tools/verify_adaptive.py`**（闭环判据） |

> **结论**：`RAW_TARGETS`、前缀拼写、`[metrics]` 字段名都属于**质量阀门的接口**。
> 改任何一处都必须同步改 `tools/`，否则 `gate_perf.py` / `verify_validity.py` /
> `verify_adaptive.py` 会静默地「解析不到 = 通过」或「解析不到 = FAIL」。
> 反过来也成立：为了让某个脚本通过而改前缀，视同篡改阀门。

## 3. `[metrics]` 行语法（脚本按字段名宽松解析）

`tools/perf_probe.py::parse_metrics` 的口径是「字段增删不会让探针失效」，
但**已有字段名与单位不许改**：

```text
win_s=<窗口秒数> screens=<屏数> blocks=<块数> loops=<循环数> frames=<渲染帧数> fps=<帧率>
idle=<空转唤醒数> avg_render_ms=<> max_render_ms=<> max_loop_ms=<>
stage_us=A<PassA> B<PassB> P<Present> C<Commit>
loop_ms_avg=msg<>/cap<>/prep<> cap_max_ms=<>
p50_ms=loop<>/cap<>/ren<> p95_ms=loop<>/cap<>/ren<>
cap_split_ms=acq<>/dirtyApi<>/copy<>/pre<>/rel<> acq_max_ms=<> rel_max_ms=<>
dirty_mpx_avg=<> dirty_mpx_max=<>
blocks_avg=<> pass_a=<>/frame pass_a_throttled=<>/frame pass_a_mpx_avg=<>
dda_acq=<>/s dda_changed=<>/s copy_mpx_avg=<>
gpu_ms_avg=A<>/B<>/T<> gpu_ms_max=A<>/B<>/T<> gpu_n=<>
```

口径要点（改之前先想清楚）：

- **5 秒墙钟窗口结算**，与「这一窗口渲染了多少帧」无关 —— 否则静止时窗口会被无限拉长，
  让 avg/max 失去可比性。
- `stage_us` 是**最后一帧**的 `[PassA, PassB, Present, Commit]` CPU 提交时长，
  **不是** GPU 执行时间；GPU 那三列来自 `gpu_timer`，只在 `GLASS_GPU_TIMINGS=1` 时非 0
  （`gpu_n` = 采样数，为 0 表示没开）。
- `p50/p95` 用逐次循环采样（上限 20000 条）算分位数，**不用均值**：
  每窗口只有几十次循环时，一次几百毫秒的离群值就能把均值彻底带偏
  （实测出现过「跳过工作反而更慢」的因果倒置）。
- `acq_max_ms` / `rel_max_ms` 是**机器健康判据**（`rel_max < 5 ms`、`acq_max < 25 ms`，
  见 `measurement-and-gates.md §1`）。

## 4. 硬规则

- **热路径禁止格式化日志。** 每帧一条 `info!` 就足以毁掉帧率与 `[metrics]` 的可比性。
  计数类诊断用计数器 + 秒级节流（`vv_last_log`）或状态翻转沿（`capture_error_logged` /
  `hidden_fs` / `hidden_empty` / `hdr_warned`）打印，见 `error-handling.md §5`。
- **不要把 `EngineEvent` 当日志用，也不要把日志当 `EngineEvent` 用。**
  上层需要程序化响应的（采集失效、档位变化、独占全屏）走事件；
  只有排障信息走 `log`。两者内容有重叠是允许的（例如采集失效既 `[warn]` 又发 `CaptureLost`），
  但**判据只能有一个来源**。
- 日志文本用中文（与项目文档一致），字段名 / 键名用英文（脚本要 grep）。
- 不要在日志里打印上层传入的完整配置（可能含窗口几何等信息，且每帧打印会刷屏）；
  需要时打 `BUILD_INFO.summary()` 这类单行摘要。