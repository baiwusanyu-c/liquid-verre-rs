# 测量方法论与质量阀门

## 1. 机器健康判据（不满足则数据作废，不得下结论）

`rel_max < 5ms`、`acq_max < 25ms`、`dda_acq ≈ 桌面实际重绘率`。
劣化签名：`rel` 200~900ms、fps 2~40、日志出现「独占全屏应用」。处置：等/关占用 GPU 的进程/注销重登，
**不要**在劣化窗口里做 A/B 或下性能结论。

## 2. 统计规则

- 一律**轮转交错 + ≥3 重复 + 中位数/p95**；均值在 10~100 loops/窗口 下无意义。
- 内存只看稳态窗口中位数；单次运行的 max 会被瞬态污染。
- 对比必须同环境相邻进行（A/B 二进制轮换），跨小时对比无效。

## 3. 确定性工具与"哪个 oracle 验什么"

| 工具/开关 | 验什么 | 注意 |
|---|---|---|
| `GLASS_SYNTH_CAP=1` | 跨进程/跨时间逐像素可比 | 合成源静态；SIM_BACKDROP 只伪造脏矩形 |
| `GLASS_CURSOR_AT` / `GLASS_FAR_GRID` | 首帧光标/分离布局，跨块隔离取证 | 首帧在显示窗口前拍出 |
| `GLASS_SHOT_EVERY_MS` | **时间维度**稳定（颤抖/闪动） | 连拍不强制全量重画 |
| `GLASS_SIM_BACKDROP=WxH@HZ` | 确定性触发节拍（吞吐测量） | **不能**用于"增量==全量"像素 oracle |
| `GLASS_SIM_BUSY=ms` | 驱动自适应控制器闭环 | 只拖拽期注入 |
| `GLASS_VERIFY_VALIDITY=1` | 金字塔虚报/漏画计数 | 计数器 oracle，最稳 |
| `verify_dirty` | 脏区书本账（增量==全量） | 真实桌面；取证帧强制全量，故测不出持久化类缺陷 |
| `repro_tremble` | 拖一块时其他块逐帧不变 | 静态合成源；排除首 2 帧与末 1 帧(teardown) |

**教训（勿重犯）**：SIM_BACKDROP 场景下做"增量==全量"像素对比是无效 oracle
（假脏区外的真实桌面变化是合理陈旧；合成源下脏区恒 [0,0,0,0] 使金字塔永不跟踪）——
该场景只能用计数器 oracle。

## 4. 阀门与阈值（`tools/gate_perf.py`）

硬门槛（FAIL=退出码 1）：单块 fps≥90 且 p50≤9ms；20 块默认档 fps≥60；
20 块 lat=2 + SIM_BACKDROP(900x700@120) fps≥90；Private<200MB。
软门槛（WARN）：20 块默认档 fps≥90、拖拽 CPU<5%（真实桌面重绘率/DWM 背压支配，非渲染成本）。
每次输出 dda_acq/dirty/own_us，用于区分"我们慢"与"桌面/DWM 忙"。

## 5. 提交前最小集合

```
cargo test --release
python tools/gate_perf.py
python tools/verify_dirty.py && python tools/repro_tremble.py far && python tools/repro_tremble.py many
python tools/verify_validity.py && python tools/verify_mouse_isolation.py && python tools/verify_adaptive.py
```
视觉改动额外：连拍目视（`view_image` 看 composed 裁切）+ 跨版本逐像素 diff 界定改动范围
（interior / ring / outside 三段统计，见 `tools/cmp_visual.py` 的思路）。
