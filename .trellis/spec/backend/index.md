# Backend Development Guidelines（tarui-glass / Rust + D3D11）

> 本目录是**本项目真实约定**的可执行契约，不是模板。改代码前先读对应文件。

## 语言约定

项目文档与代码注释使用**中文**（标识符/日志键名用英文）。本 index 原模板要求英文，
此处按项目实际约定修订：与 README / BASELINE.md / performance_report.md / 代码注释保持一致。

## Guidelines Index

| Guide | 内容 | 状态 |
|-------|------|------|
| [Directory Structure](./directory-structure.md) | 模块划分与职责边界 | ✅ 已填 |
| [Rendering Pipeline](./rendering-pipeline.md) | 逐帧管线契约 + 9 条不可破坏的不变量 | ✅ 已填 |
| [Graphics / D3D11 Gotchas](./graphics-d3d11-gotchas.md) | D3D11/DXGI/Windows 的静默陷阱清单 | ✅ 已填 |
| [Measurement & Gates](./measurement-and-gates.md) | 本机测量方法论 + 质量阀门阈值 | ✅ 已填 |
| [Error Handling](./error-handling.md) | Result<String>、退避、warn-once、不可信输入 | ✅ 已填 |
| [Quality Guidelines](./quality-guidelines.md) | 禁止/必须模式、测试要求 | ✅ 已填 |
| [Logging Guidelines](./logging-guidelines.md) | 日志前缀语义与 metrics 行语法 | ✅ 已填 |
| [Database Guidelines](./database-guidelines.md) | 不适用（无数据库），说明配置去向 | ✅ 已填 |

## 上位文档

- `BASELINE.md`（仓库根）：重构基线快照、指标表、回归命令、不变量清单
- `performance_report.md`（仓库根）：第一~十四轮全部取证与决策记录（含被否决的方案）
- `折射渲染方案.md`（用户桌面，v1.1）：需求与架构来源；偏离项在 BASELINE.md 列明
