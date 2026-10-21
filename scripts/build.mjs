#!/usr/bin/env node
// scripts/build.mjs —— 构建 workspace。
//
//   node scripts/build.mjs            # release（默认，性能阀门要求 release 档）
//   node scripts/build.mjs --debug
//   node scripts/build.mjs --only liquid-verre-rs

import { run } from "./_lib.mjs";

const args = process.argv.slice(2);
const debug = args.includes("--debug");
const onlyIdx = args.indexOf("--only");
const only = onlyIdx >= 0 ? args[onlyIdx + 1] : null;

const cargoArgs = ["build", "--workspace"];
if (!debug) cargoArgs.push("--release");
if (only) {
  // --workspace 与 --package 互斥，指定包时去掉前者
  const i = cargoArgs.indexOf("--workspace");
  cargoArgs.splice(i, 1, "--package", only);
}
process.exit(run("cargo", cargoArgs));
