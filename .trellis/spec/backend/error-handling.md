# 错误处理约定

- 统一 `Result<_, String>`，错误文本带上下文（`format!("Create DDA texture: {e}")`）；不引入 thishow/anyhow。
- **渲染主循环不得 panic**：采集错误走退避（500ms）+ 隐藏画布 + 恢复日志；RT 创建失败向上返回 Err 让该屏降级。
- **warn-once 模式**：高频路径上的告警必须有 `*_logged`/`*_warned` 布尔位（`capture_error_logged`、`hdr_warned`），
  避免日志洪水。
- **不可信输入先 sanitize**：DDA 脏/移动矩形、面板数值、环境变量解析（`parse().ok()` + 默认值 + clamp）。
  整数运算在 debug 下溢出即 panic ⇒ 外部整数先 clamp 或 widen 到 i64（`sanitize_rect` 有单测）。
- **时间运算**：`Instant` 相减一律 `saturating_duration_since` / `checked_sub`；禁止裸 `-`（开机时间短会 panic，有前科）。
- 环境变量开关解析失败 = 关闭该开关（默认安全），不报错。
- GPU 调用失败（hazard 类）**没有错误码**：靠代码审查 + `graphics-d3d11-gotchas.md` 清单防御，
  不靠运行时检测。
