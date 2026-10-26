# Implementation Report — P1 AttachThreadInput + 归因日志 + FR3 结案

日期: 2026-10-26  状态: 代码完成，待最终桌面取证补齐

## 改动文件清单

| 文件 | 改动 |
|------|------|
| `crates/liquid-verre/src/api/handle.rs` | `Command::SetFocusHwnd{id,hwnd}` + `GlassWindow::set_focus_hwnd` + 2单测 |
| `crates/liquid-verre/src/runtime/app.rs` | `block_focus_hwnds` + `focus_guard_warned` + `focus_attach_warned` + `focus_skip_warned` + `focus_hwnd_for()` |
| `crates/liquid-verre/src/runtime/commands.rs` | apply_one 守卫 + 增删改 + destroy 清理 |
| `crates/liquid-verre/src/runtime/mod.rs` | App init 补齐四个 warn-once 位 |
| `crates/liquid-verre/src/platform/window.rs` | AttachThreadInput FFI + `focus_set` ret/path/tid 三态日志 + `focus_skip` 归因 + ESC 注释 |
| `playground/src/main.rs` | GetConsoleWindow/GetCurrentThreadId/CreateWindowExW FFI + `focus_cmd` 归因日志 + set_focus_hwnd |
| `tools/verify_focus_drop.py` | 新建 v5: FR3 50轮前置轮询 + FR1 归因日志解析 + --only 拆分 |
| `README.md` | IDropTarget 小节 + best-effort focus 契约 |

## 闸门

| 检查项 | 结果 |
|--------|------|
| cargo test --release --workspace | 243 passed |
| cargo clippy -D warnings | clean |
| cargo fmt --check | clean |
| node scripts/check.mjs | 全绿 |

## FR3 实测（五连取证）

| 指标 | 数值 |
|------|------|
| 轮数 | 50 |
| 块内采样点 | 9（四角内缩32 + 中心 + 四边中点） |
| 前置轮询 | 5ms步进, 400ms超时 |
| 命中率 | 4500/4500 = 100.0% (×5) |
| 首次命中延迟 | 0.1-0.8ms, 中位 0.2ms |
| 稳定漏检 | 0 |
| PID归因/进程自洁 | 已固化 |

## FR1 归因日志样本（编译到位，待运行取证）

```
[verify] focus_cmd target=0x%x           # playground 调 set_focus_hwnd
[verify] focus_set block=% target=0x%x ret=0x%x path=direct|attached|deny tid=%
[verify] focus_skip block=% (sink present, no focus_hwnd stored)
```

## FR1 焦点路由机制

- AttachThreadInput 标准手法：当前线程==目标线程→direct SetFocus；否则 attach→SetFocus→detach（微秒级）
- attach 失败→deny（warn-once, 不重试, 不持锁）
- README best-effort 契约：宿主兜底=PostMessage 自 SetFocus

## 桌面回归

| 测试 | 状态 |
|------|------|
| verify_input_sink | PASS |
| verify_dirty | PASS |
| verify_mouse_isolation | PASS |
| verify_validity | PASS |
| verify_adaptive | PASS |
| verify_content | PASS (5/5) |
| cmp_visual single | ⚠ 待独立会话复认（前期PASS四段0，无像素改动） |
| gate_perf A/B | ⚠ 待独立会话 |

## 待办

1. 独立会话跑 cmp_visual single（四段0复认）+ gate_perf A/B + 连击100压力
2. FR1 归因日志现场取证（需同会话构建+运行，沙箱路径映射限制）
