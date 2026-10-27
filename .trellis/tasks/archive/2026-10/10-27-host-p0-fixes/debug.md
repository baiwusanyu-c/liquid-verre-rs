# Debug: verify_focus_drop FR1 红 —— 环境漂移，非本任务回归（2026-10-27）

现象：修复版跑 `verify_focus_drop.py`，FR1「FocusHint 行数=0 / focus_applied ret≠0」FAIL。

定位链：
1. 库内 FocusHint 只在 `app.focus_hwnd_for(hit_id)` 有登记时发出；日志显示
   `[verify] focus_cmd target=0x0` —— playground 的焦点代表窗口注册成了 0。
2. playground 焦点 demo 取柄顺序 = GetConsoleWindow() → 退化 1×1 STATIC 壳窗口。
   当前 Codex exec 会话的 subprocess **无 console**（GetConsoleWindow=0），
   壳窗口 `CreateWindowExW` 在 playground 进程里也返回 0（同参数在 python 进程直测
   却创建成功 —— 差异未归因，不影响本案结论）。
3. **A/B 实锤**：`git stash` 库改动 → 重建基线 0.5.0 → 同脚本 FR1 同样 FAIL；
   恢复改动重建 → FR3/防冻结心跳/连击100 与基线逐项一致。

结论：FR1 红 = 本机执行环境（无 console）相对 2026-10-26 发版时环境的漂移，
与 #1~#5 修复无关。**发版闸门处置**：本轮以「基线同红 + 其余断言全过」记录为
等价绿；若要恢复 FR1 取证能力，playground 壳窗口 fallback 需要单独排查
（登记为后续任务候选，不在本任务扩 scope）。


# Debug 2：trellis-check 子代理中断留下的破坏实验残局（2026-10-27，主代理现场恢复）

**现象**：check 代理（Locke）被中断后，工作区编译直接失败：
- `E0425 super::is_class_already_exists` ×11 —— window.rs 的判别力测试被重复插入了
  **12 份**（`_patch_block.py` 式的 `rfind brace` 插入把测试块散布到模块作用域各处，
  只有 `mod tests` 内那份合法）。
- `E0124 field scale already declared` —— BlockGeom 里 `pub scale: f32,` 出现两行。
- register_class 调用位被**回滚回 0.5.0 旧 bug**（`!= 1410`）且未恢复 —— 这正是 check
  清单第 6 条「破坏实验：改坏→看红→恢复」做到一半被打断的状态。
- BlockGeom.doc 注释被罗马化成拼音、rehome_block doc 里 `rects` 断行成裸代码行
  （编译期 `\u{ff1b}` 报错）。

**恢复动作**（全部现场修复并复验）：
1. 删 11 个散落测试块（保留 mod tests 内唯一一份）；
2. register_class 调用位恢复 `!is_class_already_exists(err.code().0)` + 前科注释；
3. BlockGeom 去重复字段、两处 doc 注释原文恢复（含「2x 膨胀」前科注释补回测试内）；
4. 复跑全链：fmt / clippy -D warnings / test --workspace 244 绿 / --ignored 两轮 PASS /
   check.mjs 全绿 / 重建 playground / verify_monitors_rebuild + verify_clickthrough_recovery
   双脚本对新 EXE 复验 PASS / 编码扫描 UTF-8 无 BOM 无 CRLF、拼音残留全文件 0 命中。

**流程教训（登记）**：派发「带破坏实验」的核查子代理时，破坏与恢复之间必须**原子**
（改坏→跑测试→立即恢复在同一批命令里完成），否则被打断就把工作区留在红态；
主代理在收到中断/超时后，第一件事应是 `cargo build` 探伤，而不是信任中断前的进度。
