#!/usr/bin/env node
// scripts/zip.mjs —— 把一个目录压成 zip（供 `cargo xtask package` 调用，也可单独用）。
//
//   node scripts/zip.mjs <sourceDir> <outZip>

import path from "node:path";
import fs from "node:fs";
import { createZip } from "./_lib.mjs";

const [src, out] = process.argv.slice(2);
if (!src || !out) {
  console.error("用法: node scripts/zip.mjs <sourceDir> <outZip>");
  process.exit(2);
}
if (!fs.existsSync(src) || !fs.statSync(src).isDirectory()) {
  console.error(`[zip] 源目录不存在: ${src}`);
  process.exit(1);
}
const result = createZip(path.resolve(src), path.resolve(out));
console.log(`[zip] ${out} <- ${result.files} 个文件, ${result.bytes} 字节`);
