# PRD —— liquid-verre-rs 库化重构

## 背景

现状：`liquid-glass` 是一个**二进制**项目（`src/main.rs` 2133 行，窗口/消息泵/采集调度/
质量控制器/面板同步/指标上报全部混在一起），控制面板（`src/ui/`，tiny_http + 浏览器页面）
与业务代码同层。上游应用无法把它当组件使用。

目标：重构为 **`liquid-verre-rs` 库项目**，为上层应用提供「液态玻璃窗口组件」；
上层应用可以创建玻璃窗口、全量配置其外观参数、在窗口之上构建自己的 UI（典型场景：应用弹窗）。
原有控制面板降级为 **dev playground**，不属于库的业务代码。

## 硬性约束（不可协商）

1. **性能不降级**：`tools/gate_perf.py` 的硬门槛必须继续 PASS
   （single-drag fps≥90 且 p50≤9.0 ms；many-drag fps≥60；lat=2+合成负载 fps≥90；Private<200 MB）。
2. **液态玻璃视觉不变**：现有 8 个回归脚本（`verify_dirty` / `verify_mouse_isolation` /
   `verify_validity` / `verify_crossscreen` / `verify_env_ema` / `verify_adaptive` /
   `verify_all` / `cmp_visual`）结论不变；默认参数下 overlay 逐像素一致。
3. **BASELINE.md 的 9 条不变量不被破坏**；改 blur kernel / scissor 必须重算幂等外扩常量。
4. 不引入 WebView / 前端运行时；不新增重量级依赖。

## 需求（工程）

- E1 合理的架构与目录划分：Cargo workspace，库 / playground / xtask 三者分离，库内部按
  职责分层（api / runtime / platform / gfx / capture / core / input / diag）。
- E2 库级日志与错误：统一 `Error` 类型（可被上层 `match`/捕获）+ `log` facade，
  上层可安装自己的 logger；无 logger 时提供默认 stderr 输出。
- E3 完全符合 Rust 规范：无 `dead_code` / `unused` 警告，`cargo clippy` 干净，公共 API 全部有文档注释。
- E4 构建/打包脚本：`cargo xtask build|check|package|gate`，外加 `scripts/*.ps1` 薄封装；
  打包产物落到 `dist/`（不含发版/部署流程）。
- E5 控制面板迁出业务代码：`src/ui/` → `playground/`（独立 crate，dev-dependency 语义）。
- E6 单元测试 + 性能指标测试齐备且可一条命令跑通。
- E7 同步更新 `.trellis/spec/`、`README.md`、`CLAUDE.md`、`AGENTS.md`、`BASELINE.md`。
- E8 项目名与文件夹名重命名为 `liquid-verre-rs`。

## 需求（功能）

- F1 **全参数配置化**：控制面板当前支持的每一项调节都必须能通过库 API 由上层传入；
  并且库支持的**全部**光学/交互参数（不只面板上那 17 个）都要可配置、可序列化。
- F2 **边缘 / 对角拖拽改变玻璃块大小**（当前只有右下角一个 grip）。
  - F2.1 鼠标移到**边缘**时：加粗白色 border 视觉反馈，border 只画到圆角切点为止，**不沿圆角拐弯**。
  - F2.2 鼠标移到**对角（圆角）**时：加粗白色 border 视觉反馈，border **沿圆角弧线拐弯**。
  - F2.3 边缘拖拽、对角拖拽可分别通过参数开启 / 禁用。
- F3 **新增「透明玻璃」模式**，与现有「液态玻璃」模式并列。
  - F3.1 透明玻璃模式的性能指标同样满足「性能需求」。
  - F3.2 透明模式只支持三个参数：圆角半径、玻璃透明度、玻璃颜色。
  - F3.3 本质 = 一个可调圆角 / 半透明度 / 颜色的玻璃块（窗口），不做折射、不做模糊。
  - F3.4 可通过参数在两种模式之间切换（运行期可切）。
- F4 **层级调节**：玻璃窗口支持「一直被覆盖（always bottom）」「一直在最前（always top）」
  「点击到最前（click to front）」三种层级语义；块之间的 z 序也需可 API 调节。
- F5 单测 / 性能测试随重构同步更新（**性能标准不变**，只改测试以适配新结构）。

## 验收标准

| # | 验收项 | 判定方式 |
|---|--------|----------|
| A1 | workspace 三个 crate 独立编译；库无 `main` | `cargo build --release --workspace` |
| A2 | 库导出完整配置 API，全参数可 serde 往返 | `cargo test --release`（新增 config 单测） |
| A3 | 错误可被上层捕获；日志走 `log` facade | 新增 error/log 单测 + playground 演示 |
| A4 | `cargo build --release` / `cargo clippy` 零警告 | xtask check |
| A5 | `cargo xtask package` 产出 `dist/` | 人工核对产物清单 |
| A6 | 8 种缩放 handle 命中判定与几何换算正确 | 新增 resize 单测（≥10 个） |
| A7 | 边缘 border 直、对角 border 沿弧；两者可分别禁用 | 连拍像素证据（`GLASS_SHOT_EVERY_MS`） |
| A8 | 透明模式只受 3 个参数影响；模式可运行期切换 | 单测 + 截图证据 |
| A9 | 透明模式性能不劣于液态模式 | gate_perf 新增 transparent 场景 |
| A10 | 三种层级语义可切换且生效 | 手工验证 + 单测（枚举/序列化） |
| A11 | 性能阀门 PASS，液态玻璃 overlay 逐位不变 | `gate_perf.py` + `verify_dirty`×3 + `cmp_visual` |
| A12 | 文档/spec 与新结构一致；文件夹已重命名 | 人工核对 + `git status` |
