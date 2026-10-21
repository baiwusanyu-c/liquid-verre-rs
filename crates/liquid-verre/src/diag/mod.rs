//! 诊断层：环境变量开关、`[metrics]` 行输出、取证截图。
//!
//! **这一层不属于库的对外契约**：所有开关默认关闭，关闭时行为与「诊断代码不存在」
//! 逐位一致。保留它们是因为 `tools/` 下的 8 个回归脚本与 `gate_perf.py` 全靠这些开关取证。

pub(crate) mod env;
pub(crate) mod metrics;
