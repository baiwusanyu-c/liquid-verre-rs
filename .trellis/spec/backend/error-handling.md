# 错误处理约定

## 1. 类型体系（`src/error.rs`）

库对外只暴露三个东西：

```rust
pub enum ErrorKind { /* #[non_exhaustive] */ }   // 10 类，见下表
pub struct Error { kind, message, source }        // Debug + Display + std::error::Error
pub type Result<T> = std::result::Result<T, Error>;
```

- 上层**按 `Error::kind()` 分支**（重试 / 降级 / 上报），**不要匹配字符串**。
- `Error::message()` 是人类可读文本（不含类别前缀）；`Display` 输出 `[kind] message[: source]`。
- 底层 HRESULT / Win32 文本保留在 `std::error::Error::source()` 里，排查时不丢信息。
- 构造只在失败分支发生（`Error::new` / `Error::with_source` 都要 `String`），**热路径零额外分配**。
- `ErrorKind` 标了 `#[non_exhaustive]`（加变体是常态）；**值类型（`*Config` / `*Params` / `*Options`）
  绝不标**（见 `quality-guidelines.md`）。

| `ErrorKind` | `as_str()` | 语义 |
|---|---|---|
| `GraphicsInit` | `graphics-init` | D3D11 设备 / DXGI 工厂创建失败（含回退 WARP 后仍失败） |
| `ShaderCompile` | `shader-compile` | HLSL 编译或着色器 / 状态对象创建失败 |
| `MonitorEnumeration` | `monitor-enumeration` | `EnumDisplayMonitors` 返回空或调用失败 |
| `WindowCreation` | `window-creation` | 画布窗口 / 窗口类创建失败（含「没有任何显示器能创建画布」） |
| `CaptureUnavailable` | `capture-unavailable` | DDA 与 WGC 都拿不到；**启动期出现即为致命** |
| `CaptureLost` | `capture-lost` | 运行期采集失效（锁屏 / 独占全屏 / 被抢占）；**不会作为 `Err` 返回** |
| `InvalidConfig` | `invalid-config` | 上层配置非法（sanitize 夹紧后仍不可用，例如尺寸为 0 / NaN） |
| `NotSupported` | `not-supported` | 当前平台 / 驱动不支持请求的能力 |
| `Shutdown` | `shutdown` | 引擎已关闭后再调用句柄方法 |
| `Internal` | `internal` | 其它未分类（也是 `From<String/&str>` 的退化目标） |

## 2. `From` 收敛策略

内部大量既有代码用 `Result<_, String>`（Win32 调用点逐个改成精确类别的收益低于成本），
因此提供四个 `From`，让 `?` 平滑收敛到 `Error`：

| From | 结果 |
|---|---|
| `String` / `&str` | `ErrorKind::Internal` + 原文 |
| `windows::core::Error` | `Internal` + `"Win32/COM 调用失败 ({code:?})"`，原错误进 `source()` |
| `std::io::Error` | `Internal` + `"I/O 失败"`，原错误进 `source()` |

**需要精确类别的点位必须显式构造 / 改写**，不要指望 `From` 猜对：

```rust
let gfx = Device::create().map_err(|e| e.with_kind(ErrorKind::GraphicsInit))?;   // Internal → GraphicsInit
return Err(Error::new(ErrorKind::MonitorEnumeration, "未枚举到任何显示器"));
return Err(Error::with_source(ErrorKind::Internal, "WinRT 初始化失败", e));       // 保留 HRESULT
```

`Error::with_kind(self, kind)` 是「把 `Internal` 细化」的唯一手段，只用于这种边界转换点。

## 3. 什么该返回 `Err`，什么该走事件

**判据：上层能不能通过重试 / 改配置解决。**

- **不可恢复的初始化失败 → `Err`**：`Engine::new` / `Engine::start` / `Engine::run` /
  `Engine::join` / `Engine::create_window` 是仅有的会返回 `Result` 的公共方法。
- **可恢复错误 → `EngineEvent`，绝不返回 `Err`**：采集失效、独占全屏让路、DPI 感知被宿主占用、
  句柄指向已销毁窗口……全部走 `EngineEvent::Warning { kind, message }` 或专用事件
  （`CaptureLost` / `CaptureRestored` / `ExclusiveFullscreen` / `MonitorsChanged` / `QualityLevelChanged`）。
  理由：这些在锁屏 / 游戏全屏期间会**每 500 ms 发生一次**，做成 `Err` 会把上层刷屏，
  而且上层根本无从处理。
- **渲染线程内的致命错误**：`log::error!` + `EngineEvent::Fatal { kind, message }` +
  `shared.set_fatal(e)`（只记第一条），随后 `EngineEvent::Shutdown`。
  上层用 `Engine::take_fatal_error()`（取出即清空）或 `join()` 的返回值拿到它。
- **句柄方法一律不返回 `Result`**：窗口可能已被销毁，setter 变成 no-op +
  一条 `Warning`（`runtime/commands.rs::warn_missing`），getter 返回 `None`。
  上层用 `GlassWindow::is_alive()` 判断。
- **`Drop` 不得 panic**：`Engine::drop` 调 `shutdown()`（幂等）；快照锁中毒时
  `read_snapshot` 退化为 `Snapshot::default()`，**不能让上层 API 直接 panic**。

## 4. 渲染主循环不得 panic

- 采集错误走退避（`capture.retry_backoff_ms`，默认 500 ms）+ 隐藏画布 + 恢复日志。
- 某屏 RT / 合成器 / 画布创建失败 → 该屏**降级跳过**（`MonitorRt::create` 返回 `None` + `[warn]`），
  其它屏照常；全部屏都失败才 `Err(ErrorKind::WindowCreation)`。
- 单块绘制失败 → `continue`（跳过该块），不中断整帧。

## 5. warn-once 语义

高频路径上的告警**必须**有布尔位或时间闸，否则日志洪水本身就会毁掉帧率与取证：

| 点位 | 机制 |
|---|---|
| 采集失效 | `MonitorRt::capture_error_logged`：状态翻转时各打一次（失效 `[warn]` / 恢复 `[info]`） |
| HDR 屏强制 DDA | `DdaCapture::hdr_warned`：一次 |
| `GLASS_VERIFY_VALIDITY` 自检计数 | `Compositor::vv_last_log`：**≥1 秒**才打一行 `[verify]`，且只在计数 > 0 时打 |
| `[metrics]` | 5 秒墙钟窗口一行（`diag/metrics.rs`），与帧数无关 |
| 独占全屏 | `MonitorRt::hidden_fs` 翻转时才打 `[info]` + 发事件 |
| 空屏隐藏 | `MonitorRt::hidden_empty` + `empty_since` 1 秒防抖 |

新增周期性告警时照抄这套：**状态位 + 翻转沿打印**，或**计数器 + 秒级节流**。
`tools/` 的取证脚本按行解析日志，刷屏会直接让 `verify_validity` / `verify_adaptive` 误判。

## 6. 不可信输入先 sanitize

- **一切来自上层的数值**：`EngineConfig` / `WindowConfig` / `GlassStyle` / `LiquidParams` /
  `TransparentParams` / `ResizeOptions` / `ResizeFeedbackOptions` 都有 `sanitize()`，
  入口（`Engine::new` / `create_window` / 各 setter / `commands::apply`）必须先调用再进 GPU。
- **`clamp_finite(v, lo, hi)` 而不是 `f32::clamp`**：`f32::clamp` 遇 NaN **原样返回 NaN**
  （两个比较都是 false），NaN 一进 GPU，SDF 会整块消失且不报任何错——极难排查。
  `clamp_finite`：NaN → `lo`，`+inf` → `hi`，`-inf` → `lo`。
- **先校验原始输入，再 sanitize**：`create_window` 先查 `rect` 是否有限、宽高是否 > 0，
  通过了才 `sanitize()`。顺序反了的话「上层传了 0×100」这种明显的编程错误会被静默夹成 24×100。
- **DDA 脏 / 移动矩形**是不可信外部输入：`sanitize_rect` 先 clamp 到屏幕、丢弃退化矩形。
  整数运算在 debug 下溢出即 panic ⇒ 外部整数先 clamp 或 widen 到 `i64`（有单测）。
- **环境变量**：`parse().ok()` + 默认值 + `clamp`；解析失败 = 关闭该开关（默认安全），不报错。

## 7. 时间运算

`Instant` 相减一律 `saturating_duration_since` / `checked_sub`；**禁止裸 `Instant - Duration`**
（越界会 panic，开机时间短时真炸过）。「很久以前」统一写成：

```rust
Instant::now().checked_sub(Duration::from_secs(3600)).unwrap_or_else(Instant::now)
```

现状点位：`core/quality.rs::far_past()`、`runtime/monitor.rs::MonitorRt::create`、
`runtime/mod.rs`（`fs_check_at`）、`gfx/compositor.rs`（`last_pass_a_at`）。

## 8. GPU 静默失败

GPU 调用失败（hazard 类：RTV 仍绑为 SRV、`Clear` 忽略 scissor、flip 后备缓冲不持久……）
**没有错误码**。靠代码审查 + `graphics-d3d11-gotchas.md` 清单防御，不靠运行时检测。
这类点位必须写注释说明「为什么这一行是安全的 / 为什么必须先解绑」。