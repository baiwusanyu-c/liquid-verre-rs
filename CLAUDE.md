# CLAUDE.md —— tarui-glass（Windows 桌面液态玻璃叠加层）

Rust + 手写 D3D11/DComp/WGC/DDA 的桌面「液态玻璃」叠加层，实现《折射渲染方案》v1.1
（用户桌面 `折射渲染方案.md`）。每屏一张全屏画布：采集 → 共享模糊金字塔(Pass A, scissor)
→ 逐块着色(Pass B) → 单 Present + DComp Commit；事件驱动，静止即 0 帧。

## 常用命令

```powershell
cargo build --release          # 产物 target/release/liquid-glass.exe
cargo run                      # debug 亦可（元数据已 sanitize，debug 不再 panic）
cargo test --release           # 单元测试（质量阀门 1）
python tools/gate_perf.py      # 性能阀门（质量阀门 2，硬门槛 FAIL 退出码 1）
python tools/verify_dirty.py   # 视觉/一致性阀门之一（共 8 个，清单见 BASELINE.md）
```

## 不可破坏的不变量（详见 BASELINE.md / .trellis/spec/backend/rendering-pipeline.md）

1. 每帧自包含：整屏清 + Pass B 画全部块（flip back buffer 不持久）；Pass A 按脏区 scissor。
2. 金字塔有效性只声称真正重算过的区域；`!covered` 先于 `backdrop_changed`。
3. 模糊链幂等外扩常量：下采样 14/12/8/0、tent 32/24/16/8、首级外接框 ⊕48。
4. 环境色 EMA 只在捕获内容真变化时推进。
5. 鼠标对非 hover 块的影响 = fall × gate(3σ)，盒外精确 0；影响盒 = `cursor_influence_px`（单一来源）。
6. 脏区三来源（桌面脏区/位移/光标盒）始终并集，不得短路。
7. HDR 输出自动切 WGC（DDA 复制面是 HDR 参考值，会过曝）。
8. 自适应控制器判据只看「我们自己的工作」(prep+A+B+Commit)，排除采集等待与 Present 阻塞。
9. DDA 元数据先 sanitize 再运算（debug 溢出即 panic 的前科）。

## 测量规则（本机结论的唯一合法来源）

- 轮转交错 A/B + ≥3 重复 + 中位数/p95；单次运行的内存/fps 数字不可信。
- 机器健康判据：`rel_max<5ms`、`acq_max<25ms`；劣化窗口的数据作废。
- oracle 选择表见 `.trellis/spec/backend/measurement-and-gates.md`
  （SIM_BACKDROP 场景禁止做"增量==全量"像素对比；时间维度用连拍）。
- 视觉取证只能看 `GLASS_INTERNAL_SHOT` 的 composed 图（WDA 排除使 OS 截屏看不到玻璃）。

## 协作约定

- 回答与文档用中文；代码标识符英文；中文注释写"为什么"并带实测数字。
- **未经用户明确要求不得 git commit / push。**
- PowerShell 写含中文文件前先 `[Console]::InputEncoding=UTF8` 且 `-Encoding utf8`；优先 Python 写文件。
- 新开关一律 `GLASS_*` 前缀、默认安全、README 登记。
- 回滚点：tag `baseline/pre-refactor`、分支 `backup/pre-refactor-baseline`、起点 `4dca041`。
