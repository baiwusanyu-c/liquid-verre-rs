#!/usr/bin/env node
// scripts/check.mjs —— 提交前的质量闸门：fmt + clippy(-D warnings) + test。
//
//   node scripts/check.mjs
//
// 三步全跑完再汇总，不会在第一步失败时就停下（这样一次就能看到全部问题）。
// 等价于 `cargo xtask check`。

import { run } from "./_lib.mjs";

const steps = [
  ["rustfmt", ["fmt", "--all", "--", "--check"]],
  ["clippy", ["clippy", "--workspace", "--all-targets", "--release", "--", "-D", "warnings"]],
  ["test", ["test", "--workspace", "--release"]],
];

const failed = [];
for (const [name, args] of steps) {
  console.log(`\n==== check: ${name} ====`);
  if (run("cargo", args) !== 0) failed.push(name);
}
if (failed.length) {
  console.error(`\n[check] 未通过: ${failed.join(", ")}`);
  process.exit(1);
}
console.log("\n[check] 全绿");
