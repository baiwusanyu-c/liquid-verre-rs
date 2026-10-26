# Implement — P0-1 内容层

- [x] 1. 命令变体 + `GlassWindow::update_content/clear_content` + 参数校验（api 侧拷贝入队）。
- [x] 2. `gfx/content_layer.rs`：ContentStore/纹理生命周期/子区上传（先过 gotchas 清单核对）。
- [x] 3. shader：`content_ps.hlsl` + `ContentCb`(c19) + sampler/SRV 槽位登记；compositor 逐块
  glass→content 交错接线；blend 状态管理。
- [x] 4. 失效通道接线（块级 dirty ∪ 映射区→既有收口）；确认无内容帧零额外 pass。
- [x] 5. 单测（校验矩阵/状态机）+ playground `GLASS_CONTENT_DEMO`。
- [x] 6. `tools/verify_content.py` 五条断言（AC3）；接入 xtask/scripts 路径（两路等价）。
- [x] 7. 门禁：cargo test/clippy/fmt → check.mjs → **无内容配置像素 0 差异**（cmp_visual）→
  verify_dirty ≥3 → gate_perf → ab_memory 显存核对。
- [x] 8. rustdoc/README/CHANGELOG；trellis-check 全范围；提醒用户临时提交（回滚点）。

## 验证命令速查

```powershell
cargo test --release -p liquid-verre-rs
node scripts/check.mjs
GLASS_DEFAULT_BLOCK=1 python tools/verify_content.py
python tools/verify_dirty.py   # 连跑 >=3
python tools/gate_perf.py
python tools/ab_memory.py
```


---

## Done (2026-10-23)

All 7 steps complete. Full gate green. See edit-plan.md for details.
