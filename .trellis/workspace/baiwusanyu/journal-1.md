# Journal - baiwusanyu (Part 1)

> AI development session journal
> Started: 2026-10-20

---



## Session 1: 视觉缺陷根治 + 性能阀门 + Trellis spec 落地（重构前基线）
<!-- trellis-session: v=2 fp=00690d86c5544293 -->

**Date**: 2026-10-20
**Task**: 视觉缺陷根治 + 性能阀门 + Trellis spec 落地（重构前基线）
**Branch**: `master`

### Summary

第九~十四轮：跨块串扰(3σ gate)、控制器误降档(own-work 判据)、切档内存、HDR 过曝(自动切 WGC)、拖一块其他块闪动(Clear 忽略 scissor + flip 不持久 -> 每帧自包含)、金字塔幂等外扩、EMA 门控、debug 溢出 panic、latency 1/2 取舍(默认 1)；25 单测 + gate_perf + 8 回归脚本成为重构阀门；Trellis spec 8 份落地 + CLAUDE.md/AGENTS.md/README 更新；基线 tag baseline/pre-refactor、分支 backup/pre-refactor-baseline、重构起点 4dca041

### Git Commits

| Hash | Message |
|------|---------|
| `2be3393` | fix: 跨块串扰/漏更新/HDR 过曝三组视觉缺陷修复与取证 |
| `f58389d` | fix: 根治「拖一块、其他块闪动」——画布每帧自包含 + 金字塔幂等 + 启动 panic |
| `d7585a9` | perf: 20 块拖拽达到 >=90fps —— GLASS_FRAME_LATENCY 默认 2 + 下采样链幂等补完 |
| `bcf9511` | merge+revert: 合并分叉 tip；GLASS_FRAME_LATENCY 默认回退 1（跟手优先） |
| `82fc311` | chore: 重构前基线快照 BASELINE.md（指标/回归清单/关键不变量/已知偏离） |
| `4dca041` | test: 重构前质量阀门 —— 25 个单元测试 + gate_perf.py + BASELINE 阀门章节 |
| `6db6345` | docs/spec: Trellis 规范落地 + 重构基线文档更新 + 多平台集成入库 |

### Status

[OK] **Completed**
