# PRD：Win10 + Wallpaper Engine 兼容修复（分类框黑框 / 点桌面玻璃消失）

## 背景（来源：宿主 chapanda-ds-organizers 定位报告 2026-11，Win10 + WE 实测）

- 症状②：点击桌面空白处 → 分类框整体消失；点任务栏又出现。根因 = 库的
  ullscreen_app_on 形状判据把 **WorkerW**（WE 挂壁纸的桌面壳窗口，样式与
  Progman 同形）误判成独占全屏应用 → hidden_fs → SW_HIDE + 整屏停产。
  0.4.2 修 Progman 误报时只挡了 GetShellWindow() 返回的那一个 HWND，
  WorkerW 是**同一个坑的第二半**。
- 症状①：液态与非液态两种风格下分类框都是黑色（共用同一张采集纹理）。
  两条候选路径：
  - A：WGC 后端会把「带 WDA_EXCLUDEFROMCAPTURE 的自家全屏画布」涂成黑矩形
    （库 capture/dda.rs 模块头自证 Win10 行为），HDR 输出 / DDA 创建失败回退
    都会落到 WGC。
  - B：DDA 复制帧不含 MPO 硬件叠加层（WE 视频壁纸在 Win10 常见），壁纸区域为黑。
- 本机（Win10、无 WE）无法直接复现 A/B 的真实触发，修复必须是
  「运行期自检测 + 自降级 + 可判别取证」，而不是按 OS 版本号猜。

## 需求

- R1 [P0] ullscreen_app_on 增加**桌面壳类名排除**（Progman / WorkerW，
  ASCII 大小写不敏感、零分配、只在便宜判据之后付 GetClassNameW）。
  宿主 is_desktop_class 同时认这两个类，库侧补齐同一常识。
- R2 [P1] 采集黑帧哨兵（会话级一次性状态机，不进热路径）：
  - 每会话采前 2 个「内容变化」帧做全黑判定（网格采样 + 摘要 digest）。
  - WGC 后端连续全黑 ⇒ 临时摘除画布采集排除（WDA_NONE）取一帧裁决：
    帧内容/摘要改变 ⇒ 判定「排除即涂黑」（路径 A）⇒ 自动降级 DDA
    （GLASS_BACKEND=wgc 或 ForceWgc 强配时只告警不降级）。
  - DDA 后端连续全黑 ⇒ **只发 Warning 事件**（ErrorKind::CaptureDegraded），
    绝不隐藏画布 —— 纯黑桌面与 MPO 黑帧在采集侧不可分辨，隐藏会把合法用户
    （黑色壁纸）打成「分类框消失」，这是与指南建议的有意分歧，必须写进 CHANGELOG。
  - 裁决帧只用于诊断，不进金字塔（防自采集污染）。
- R3 [P1] 取证后门 GLASS_BLACK_PROBE=0|force（0=禁用；force=「排除在画布上的帧」
  一律视为全黑，用于本机无 WE/无 HDR 也能闭环验证状态机、affinity 翻转与降级）。
- R4 [P2] ullscreen_hide 文档口径更新（0.4.2 Progman + 0.5.4 WorkerW 已排除）。

## 验收（判别力设计：删实现必须变红）

- AC1 单测：is_desktop_shell_class 认 WorkerW/Progman 及大小写变体，
  不认 WorkerW2/Progmanx/空串/越界 n。
- AC2 取证 	ools/verify_desktop_foreground.py：本机注册类名 WorkerW / Progman 的
  铺满屏 layered WS_POPUP 前台窗口 ⇒ 不得出现「检测到独占全屏应用」且 dda_acq 不归零
  （layered 排除 B1 归因）；对照组类名 VerreFakeFS 同形状 ⇒ 必须出现 marker。
- AC3 取证 	ools/verify_black_probe.py：GLASS_BACKEND=wgc GLASS_BLACK_PROBE=force
  ⇒ 必须出现降级 marker 且后端变 dda；GLASS_BLACK_PROBE=0 同刺激 ⇒ 必须无 marker；
  force + dda ⇒ 只出 CaptureDegraded Warning、后端不变。
- AC4 既有回归：erify_occlusion.py S4-S7（真全屏 marker 不许被误伤）、
  erify_monitors_rebuild.py、全量 cargo test / clippy / fmt / node check.mjs。

## 核对记录（2026-11-25 实施完毕，未提交）

- AC1：`is_desktop_shell_class` 单测 9 断言（WorkerW/PROGMAN 大小写 / Progman2 /
  Progm / 空串 / 自家类名 / 钳位）全绿；库单测 265 全绿。
- AC2：`verify_desktop_foreground.py` 五阶段 PASS（S1 真全屏仍命中、S3/S4 假桌面壳
  不命中、画布不隐藏）。首轮运行当场抓到**副屏 shell island 越屏误伤**，
  顺带修掉（fullscreen_app_on 改收画布 HMONITOR，按屏取矩形）。
- AC3：`verify_black_probe.py` R1-R4 全 PASS（force 仿真在真实 wgc/dda 上闭环；
  其中 R2 首轮 FAIL 暴露「GLASS_BACKEND 后门压过降级」→ 新增 downgrade_to_dda 绕开）。
- AC4：`cargo test --release --workspace` / clippy -D warnings / fmt --check /
  `node scripts/check.mjs` 全绿；`verify_occlusion.py`、`verify_monitors_rebuild.py`
  复跑 PASS（停产门与重建路径无回归）。
- 有意分歧登记：指南建议 DDA 连续全黑上抛 CaptureLost 类事件，实现改为
  `Warning{CaptureDegraded}` **不隐藏画布**（纯黑桌面不可分辨，隐藏误伤合法用户），
  已写进 CHANGELOG 与 error.rs 文档。
- 宿主侧动作（不在本任务）：Win10+WE 的 `fullscreen_hide` 条件默认、设置页文案、
  复现包（日志 `采集后端:` / `检测到独占全屏` 翻转沿 + WE 版本 + 壁纸类型 + 驱动）。

## 非目标

- 不改 shader/cbuffer/金字塔；不改 EngineConfig 默认值语义（fullscreen_hide 仍默认开）；
  宿主侧「Win10 默认关 fullscreen_hide」属宿主仓库，不在本任务。

- AC4 性能阀门（gate_perf.py，同机 A/B 相对档 vs 0.5.3 参考臂）：**本机未获健康窗口，待重启后重跑**。
  实测过程（2026-11-25）：本机后台 Weixin/Feishu/headless_shell 抢 GPU，spec §1 健康判据
  （rel_max < 5ms）无法满足，rel_max 全天在 9~1620ms 振荡；跑了 4 轮全量闸门 + 1 轮配对
  own_us 排除证据，臂内散布 40~180%，**方向轮间翻转**（首轮新臂更快判 PASS，后三轮参考臂
  更快、脚本按自身噪声护栏判「结果不可信」拒签）⇒ 按 §1.2 属「阈值型判据一律作废」窗口，
  既不能签 PASS，也未坐实回归。机理预判：稳态热路径本次只新增每屏每圈一次
  Instant::now + 枚举比较（探针回读仅在会话头 ≤3 个 changed 帧；GLASS_SYNTH_CAP 自动跳过），
  无每帧分配；配对观察 own_us 新臂 54~110us vs 参考臂 76~97us，交叠在散布内，方向互有胜负。
  **重启后立即重跑（越快越健康，spec：重启后 ~5min 窗口可用）**：

  ```powershell
  cd D:\github-project\liquid-verre-rs
  $env:VERRE_OLD_EXE="D:\github-project\lv-053-ref\target\release\liquid-verre-playground.exe"
  python tools/gate_perf.py        # 默认 REPS=3 x 10s，约 8 分钟；期间不要构建/跑重 GUI
  ```

  参考臂 = git worktree `lv-053-ref`（HEAD=8f18a6c release:0.5.3，未含本任务改动）。

## 性能阀门实测（2026-11-25 重启后，健康窗口）

重启后 ~10 分钟健康窗口内（散布 <3%，方向与绝对值互证）：

- **gate_perf.py（A/B 相对档，vs lv-053-ref 0.5.3 参考臂）：PASS（退出码 0）**。
  single-drag 115.5 vs 114.8（-0.7%）、many-drag 58.9 vs 59.6（+1.3%）、
  lat2 102.7 vs 104.5（+1.7%）、transparent 96.6 vs 97.0（+0.5%），全部 <3% 门槛，
  臂内散布 0.3~2.7%（对照劣化窗口的 40~180%，本窗口判据有效）；
  priv 173/182/182/121 MB 全部 <200MB 硬门槛。绝对值同步达标（p50 8.33ms 锁相）。
- **ab_latency.py：通过**。lat1/lat2 形态与历史一致（many-drag fps +35%、
  single-drag 零变化），无异常副作用。
- **ab_memory.py：通过**。full 165.5 / half 129.9 MB（3 遍重复散布 ±0.5MB，差 -35.6MB）。
- **verify_dirty 2×PASS**（第 3 次跑在劣化窗口「输出结果不足」= 样本不足，非像素判据红）；
  **verify_validity PASS**（虚报 0 / 漏画 0）、**verify_mouse_isolation PASS**（3 臂 0 差异）、
  **repro_tremble many PASS、far PASS（重跑帧数 16 达标）**。
- **verify_adaptive：结果与窗口绑定，非代码差异**。新臂 1×PASS（完整序列
  「→1 →2 ↑1 ↑0」+ Private -50.9MB 方向一致）+ 3×FAIL（劣化窗口）；
  **对称配对**中参考臂（不含本任务代码）同窗 FAIL（REF_A PASS / REF_B FAIL /
  NEW_B FAIL / NEW_C FAIL 交替）⇒ 失败归因机器窗口（spec §1.2「阈值/帧数型在劣化
  窗口一律作废」），无证据显示与本次改动相关。

待办（可选，非红线）：下次重启后 5 分钟窗口内补跑 `python tools/verify_adaptive.py`
（新臂）与 `python tools/repro_tremble.py far` 各 1 次做双确认；本任务未触碰
blur kernel/scissor/quality 逻辑，稳态热路径新增仅「每屏每圈一次枚举比较 +
Instant::now」，机理与实测一致。

