# 日志约定（eprintln → stderr）

前缀语义（取证脚本依赖这些前缀 grep）：

| 前缀 | 用途 | 示例 |
|---|---|---|
| `[boot]` | 启动阶段耗时 | `[boot] 着色器就绪 (+199 ms)` |
| `[info]` | 状态跃迁（一次性） | 采集后端、色彩空间、画布显示/隐藏、HDR 切换、自适应档位 |
| `[warn]` | 降级/回退/异常但可继续 | DDA 失效、WGC 回退、HDR 强制 dda |
| `[verify]` | 取证落盘 | `saved internal glass frame: …` |
| `[dbg]`/`[dbg-bb]` | 调试开关门控（GLASS_DBG_* / GLASS_VERIFY_VALIDITY） | 漏更新计数 |
| `[metrics]` | `GLASS_METRICS=1` 的 5s 窗口行 | 见下 |
| `[adaptive]` | 质量档位跃迁 | `降级 -> 1（…丢 vblank 40%）` |

`[metrics]` 行语法（脚本解析依赖，改字段需同步 tools/）：
`win_s / screens / blocks / loops / frames / fps / idle / avg_render_ms / max_render_ms / max_loop_ms /
stage_us=A{PassA} B{PassB} P{Present} C{Commit} / loop_ms_avg=msg/cap/prep / cap_max_ms /
p50_ms=loop/cap/ren / p95_ms=… / cap_split_ms=acq/dirtyApi/copy/pre/rel / acq_max_ms / rel_max_ms /
dirty_mpx_avg / dirty_mpx_max / blocks_avg / pass_a=/frame / pass_a_throttled / pass_a_mpx_avg /
dda_acq / dda_changed / copy_mpx_avg / gpu_ms_*`。

规则：热路径禁止格式化日志（每帧 eprintln 会毁掉帧率）；计数类诊断用原子量 + 秒级节流打印。
