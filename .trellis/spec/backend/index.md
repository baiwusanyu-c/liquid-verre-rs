# Backend Development Guidelines（liquid-verre-rs / Rust + D3D11）

> 本目录是**本项目真实约定**的可执行契约，不是模板。改代码前先读对应文件。
>
> 仓库已从单 bin crate `liquid-glass` 重构为 **Cargo workspace 库项目** `liquid-verre-rs`：
> 库 crate 在 `crates/liquid-verre/`（package 名 `liquid-verre-rs`，**lib 名 `liquid_verre`**），
> dev 控制面板在 `playground/`，任务 runner 在 `xtask/`，Node 构建脚本在 `scripts/`。
> 本目录下所有 `src/...` 路径均相对 `crates/liquid-verre/`，除非显式写出
> `playground/` / `xtask/` / `scripts/` / `tools/`。

## 语言约定

项目文档与代码注释使用**中文**（标识符/日志键名用英文）。本 index 原模板要求英文，
此处按项目实际约定修订：与 README / CHANGELOG / BASELINE.md / performance_report.md / 代码注释保持一致。

## Guidelines Index

| Guide | 内容 | 状态 |
|-------|------|------|
| [Directory Structure](./directory-structure.md) | workspace 目录树 + 每个模块的职责与边界规则（`api/` 唯一公共面、`core/` 零 Win32、`unsafe` 分布、命令队列约束） | ✅ 已按重构重写 |
| [Rendering Pipeline](./rendering-pipeline.md) | 逐帧管线契约（帧首消费命令队列、全透明屏整条跳过 Pass A）+ cbuffer `c0..c18` 语义表 + 独立 border pass（`BorderCb`） + 9 条不可破坏的不变量 | ✅ 已按重构更新 |
| [Graphics / D3D11 Gotchas](./graphics-d3d11-gotchas.md) | D3D11/DXGI/Windows 的静默陷阱清单 | ✅ 已填（重构不改变这些陷阱） |
| [Measurement & Gates](./measurement-and-gates.md) | 本机测量方法论 + 质量阀门阈值 + workspace 版命令 + 被测 EXE 路径与覆盖变量 + `cmp_visual.py` 新口径 | ✅ 已按重构更新 |
| [Error Handling](./error-handling.md) | `Error` / `ErrorKind` / `Result` 体系、`From` 收敛策略、可恢复错误走 `EngineEvent::Warning`、warn-once、不可信输入 sanitize | ✅ 已按重构重写 |
| [Quality Guidelines](./quality-guidelines.md) | 禁止/必须模式、测试要求（值类型不加 `#[non_exhaustive]`、`checked_sub`、面板代码不进库、新参数必须进 `playground/src/schema.rs`） | ✅ 已按重构更新 |
| [Logging Guidelines](./logging-guidelines.md) | `log` facade + `target:` 决定前缀（`RAW_TARGETS`）+ 「改前缀 = 打断质量阀门」+ tools 各脚本依赖哪个前缀 | ✅ 已按重构重写 |
| [Database Guidelines](./database-guidelines.md) | 不适用（无数据库），说明配置去向 | ✅ 已填 |

## 上位文档

- `README.md`（仓库根）：**面向上层应用开发者**的定位、安装、最小示例、完整 API 速查、
  线程模型契约、构建/检查/打包/阀门命令、目录树、性能指标表、`GLASS_*` 环境变量全表。
- `CHANGELOG.md`（仓库根）：`0.4.0` 库化重构的 Added / Changed / Fixed / Removed。
- `BASELINE.md`（仓库根）：重构**前**的基线快照、指标表、回归命令、9 条不变量。
  注意其中「构建 / 运行」一节仍是重构前口径（`target/release/liquid-glass.exe`）；
  重构后的命令以 README 与 `measurement-and-gates.md` 为准。
- `performance_report.md`（仓库根）：第一~十四轮全部取证与决策记录（含被否决的方案）。
- `折射渲染方案.md`（用户桌面，v1.1）：需求与架构来源；偏离项在 BASELINE.md 列明。