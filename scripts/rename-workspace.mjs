#!/usr/bin/env node
// scripts/rename-workspace.mjs —— 把仓库目录从 tarui-glass 重命名为 liquid-verre-rs（需求 E8）。
//
//   node scripts/rename-workspace.mjs            # 立即尝试，失败则提示
//   node scripts/rename-workspace.mjs --retry 60 # 每 2 秒重试，最多 60 次
//
// 为什么需要重试：Codex 桌面应用的集成终端 / 文件监听器会持有仓库根目录的句柄，
// Windows 下目录被打开（且未共享 delete）时 Rename-Item / rename(2) 会失败。
// 关掉占用它的终端标签（或在里面 `cd` 出去）后本脚本即可成功。
//
// 本脚本只做「重命名 + 校验」，不改任何文件内容：仓库内没有硬编码的绝对路径
// （tools/verify_all.py 的 BASE_EXE 指向仓库**外**的旧基线 worktree，刻意保留）。

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const parent = path.dirname(repoRoot);
const from = repoRoot;
const to = path.join(parent, "liquid-verre-rs");

const args = process.argv.slice(2);
const retryIdx = args.indexOf("--retry");
const retries = retryIdx >= 0 ? Number(args[retryIdx + 1] || 60) : 1;

if (path.basename(from) === "liquid-verre-rs") {
  console.log("[rename] 已经是 liquid-verre-rs，无需操作");
  process.exit(0);
}
if (fs.existsSync(to)) {
  console.error(`[rename] 目标已存在: ${to} —— 请人工确认后再继续，本脚本不会覆盖`);
  process.exit(1);
}

for (let i = 0; i < retries; i++) {
  try {
    fs.renameSync(from, to);
    console.log(`[rename] OK: ${from}\n            -> ${to}`);
    console.log("[rename] 请在 Codex 里重新打开新目录 D:\\github-project\\liquid-verre-rs");
    console.log("[rename] 然后跑一次: node scripts/check.mjs && python tools/cmp_visual.py");
    process.exit(0);
  } catch (e) {
    if (i + 1 >= retries) {
      console.error(`[rename] 失败（${e.code ?? e.message}）：目录仍被占用。`);
      console.error("         常见占用方：Codex 应用的集成终端（cwd 停在仓库根）或文件监听器。");
      console.error("         处理：在那个终端里执行 `cd ..`（或关掉该终端标签），然后重跑本脚本；");
      console.error("         或用 `node scripts/rename-workspace.mjs --retry 120` 让它自己等。");
      process.exit(1);
    }
    process.stderr.write(`[rename] 第 ${i + 1} 次尝试被占用，2s 后重试...\r`);
    await new Promise((r) => setTimeout(r, 2000));
  }
}
