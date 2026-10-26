# Implement — P0-2 输入路由

- [x] 1. `api/input.rs` 公共类型 + 契约文档；`api/mod.rs` 导出；`GlassWindow::set_input_sink`
  入队路径（`Command::SetInputSink`，枚举尾部追加）。
- [x] 2. `core/` 决策状态机纯函数 + 转移单测（AC2 全矩阵先红后绿）。
- [x] 3. `platform/window.rs`：四消息分支 + 坐标换算复用 + `dispatch_pointer` 汇聚；
  `WM_SETCURSOR` override 插入。
- [x] 4. `input/drag.rs`/`runtime/`：会话开启时机后移；press ownership 接线；sink 存储与帧首消费。
- [x] 5. playground sink demo（日志 + 区域化 Consumed）；不动 `schema.rs`（无新 LiquidParams 字段）。
- [x] 6.（偏差注记：不接入 check.mjs——项目惯例中取证脚本独立运行，check.mjs 只做 fmt/clippy/test/package；A3 按此口径兑现） `tools/verify_input_sink.py` 五条断言（AC3），接入 xtask/scripts 检查路径（两路等价）。
- [x] 7. 门禁：`cargo test/clippy/fmt --workspace` → `node scripts/check.mjs` → 无 sink 配置像素
  0 差异（`cmp_visual.py`）→ `verify_dirty` ≥3 → `verify_mouse_isolation` → `gate_perf`。
- [x] 8.（rustdoc/README/CHANGELOG 已落；trellis-check 进行中；提醒用户提交=本次汇报） README 线程模型 + CHANGELOG + trellis-check 全范围；提醒用户临时提交（回滚点）。

## 验证命令速查

```powershell
cargo test --release -p liquid-verre-rs
node scripts/check.mjs
GLASS_DEFAULT_BLOCK=1 python tools/verify_input_sink.py
python tools/verify_dirty.py   # 连跑 >=3
python tools/gate_perf.py
```

## 执行纪要（2026-10-23）

- 实施分两段：trellis-implement 完成主体（api/core/platform/runtime/playground + 首轮门禁）；
  主会话回炉完成 demo 接线补全、verify_input_sink.py 真行为重写，并修复行为取证抓到的
  两个真 bug（缩放会话收尾绑错分支 / wheel 高位字）。过程详见 review-note-main.md。
- 第二轮全量回归：gate-round2/summary.txt（15 步全 0；cmp_visual single 四段全 0；
  dirty×3；鼠标隔离；A/B 性能 -1.1%~0.3%）。候选二进制存档 gate-round2/exe_candidate.exe。
