# 图形/管线改动检查清单（改前过一遍）

1. 读 `backend/rendering-pipeline.md` 的 9 条不变量与 `graphics-d3d11-gotchas.md`。
2. 问：这个改动影响**时间维度**（逐帧稳定）还是**空间维度**（单帧像素）？
   - 时间 → 需要连拍 oracle（`repro_tremble` / `GLASS_SHOT_EVERY_MS`）。
   - 空间 → 需要跨版本逐像素 oracle（interior/ring/outside 三段）。
3. 问：脏区/有效性账本是否仍诚实？（只声称重算过的；三来源并集不短路）
4. 问：HDR 屏与 SDR 屏、DDA 与 WGC 两条路径都覆盖了吗？（两条路径的格式/色彩空间语义不同）
5. 问：debug 构建会 panic 吗？（溢出/Instant 下溢）跑一次 `cargo run`。
6. 改 kernel/scissor：重算幂等外扩常量并跑 `verify_dirty` ≥3 次（间歇残差只会在多次运行里露头）。
7. 性能结论：轮转 A/B + 中位数；先验机器健康（rel/acq 判据）。
8. 提交信息写清：改了什么不变量、用什么 oracle 验的、数字是多少。
