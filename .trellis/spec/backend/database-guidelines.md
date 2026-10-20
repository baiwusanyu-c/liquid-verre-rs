# Database Guidelines —— 不适用

本项目无数据库、无 ORM、无迁移。持久化现状与去向：

- 运行期状态：全内存（`Params`/`Block`/质量控制器），面板经原子量共享。
- 用户配置：控制面板 → `Params`（TOML/注册表持久化**尚未实现**，属 M2 交付物缺口，见 BASELINE 偏离清单）。
- 取证产物：`tools/*.bmp|png|json|log`（gitignore），结论抄录进 `performance_report.md`。

若将来引入配置持久化：schema 变更视同跨层契约，按 trellis-update-spec 的 7 节深度写 spec，
并补"损坏配置回退默认值"的单测。
