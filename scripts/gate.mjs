#!/usr/bin/env node
// scripts/gate.mjs —— 性能硬门槛（tools/gate_perf.py）。
//
//   node scripts/gate.mjs
//
// 阈值与判定逻辑都在 gate_perf.py 里，这里只负责「先构建、再跑、透传退出码」，
// 避免同一个阈值在 Node 和 Python 里各写一份而漂移。
//
// 注意：性能结论只在机器健康窗口下有效（rel_max<5ms、acq_max<25ms），
// 桌面重绘率低的时候 fps 会被触发源节拍锁死，那是环境问题不是回归。

import path from "node:path";
import fs from "node:fs";
import { REPO_ROOT, pythonExe, run } from "./_lib.mjs";

if (run("cargo", ["build", "--release", "--workspace"]) !== 0) {
  console.error("[gate] 构建失败，跳过性能测量");
  process.exit(1);
}
const script = path.join(REPO_ROOT, "tools", "gate_perf.py");
if (!fs.existsSync(script)) {
  console.error(`[gate] 找不到性能阀门脚本: ${script}`);
  process.exit(1);
}
process.exit(run(pythonExe(), [script, ...process.argv.slice(2)]));
