<!-- TRELLIS:START -->
# Trellis Instructions

These instructions are for AI assistants working in this project.

This project is managed by Trellis. The working knowledge you need lives under `.trellis/`:

- `.trellis/workflow.md` — development phases, when to create tasks, skill routing
- `.trellis/spec/` — package- and layer-scoped coding guidelines (read before writing code in a given layer)
- `.trellis/workspace/` — per-developer journals and session traces
- `.trellis/tasks/` — active and archived tasks (PRDs, research, jsonl context)

If a Trellis command is available on your platform (e.g. `/trellis:finish-work`, `/trellis:continue`), prefer it over manual steps. Not every platform exposes every command.

If you're using Codex or another agent-capable tool, additional project-scoped helpers may live in:
- `.agents/skills/` — reusable Trellis skills
- `.codex/agents/` — optional custom subagents

Managed by Trellis. Edits outside this block are preserved; edits inside may be overwritten by a future `trellis update`.

<!-- TRELLIS:END -->

# 项目约定（tarui-glass；Trellis 块之外，手工维护）

- 回答与文档用中文；代码标识符英文。
- 未经用户明确要求不得 git commit / push。
- 提交前必须全绿：`cargo test --release`、`python tools/gate_perf.py`、BASELINE.md 的 8 个回归脚本。
- 视觉/管线改动必须附像素级或连拍证据；oracle 选择见 `.trellis/spec/backend/measurement-and-gates.md`。
- 不要破坏 BASELINE.md 的 9 条不变量；改 blur kernel/scissor 必须重算幂等外扩常量并连跑 verify_dirty ≥3 次。
- 性能结论只在机器健康窗口下（`rel_max<5ms`、`acq_max<25ms`）给出；用轮转 A/B + 中位数。
- PowerShell 写含中文文件前设 UTF-8 且 `-Encoding utf8`；优先用 Python 写文件。
- D3D11/DXGI 静默陷阱清单：`.trellis/spec/backend/graphics-d3d11-gotchas.md`（hazard 丢绘制、
  Clear 忽略 scissor、flip 不持久、windows-0.39 DuplicateOutput1 绑定错误、HDR 复制面语义）。
- 回滚点：tag `baseline/pre-refactor`、分支 `backup/pre-refactor-baseline`、重构起点 `4dca041`。
