#!/usr/bin/env node
// scripts/package.mjs —— 构建并把产物收集到 dist/，附 build-info.json，压成 zip。
//
//   node scripts/package.mjs                 # dist/liquid-verre-rs-<ver>-<triple>.zip
//   node scripts/package.mjs --out build/out # 换输出目录
//   node scripts/package.mjs --no-zip        # 只留目录，不压缩
//   node scripts/package.mjs --debug         # 打 debug 档（自查用）
//
// 只做「打包构建」，不涉及发版 / 部署（按需求）。

import path from "node:path";
import fs from "node:fs";
import {
  REPO_ROOT, crateVersion, targetTriple, gitInfo, run, profileDir, createZip,
} from "./_lib.mjs";

const args = process.argv.slice(2);
const debug = args.includes("--debug");
const noZip = args.includes("--no-zip");
const outIdx = args.indexOf("--out");
const outRoot = path.resolve(REPO_ROOT, outIdx >= 0 ? args[outIdx + 1] : "dist");

if (run("cargo", ["build", debug ? "--workspace" : "--workspace", ...(debug ? [] : ["--release"])]) !== 0) {
  console.error("[package] 构建失败");
  process.exit(1);
}

const version = crateVersion();
const triple = targetTriple();
const profile = profileDir(!debug);
const binDir = path.join(REPO_ROOT, "target", profile);
const stage = path.join(outRoot, `liquid-verre-rs-${version}-${triple}`);

fs.rmSync(stage, { recursive: true, force: true });
fs.mkdirSync(stage, { recursive: true });

/** 收集产物。返回 [{ rel, abs }]。 */
function collect() {
  const out = [];
  const push = (rel, abs) => {
    if (fs.existsSync(abs)) out.push({ rel, abs });
  };
  // 库产物：rlib 一定有；cdylib / staticlib 只在开启对应 crate-type 时存在。
  for (const n of ["libliquid_verre.rlib", "liquid_verre.dll", "libliquid_verre.a"]) {
    push(`lib/${n}`, path.join(binDir, n));
  }
  // dev playground：不是发布物，但一起给出便于验证产物可用性。
  push("bin/liquid-verre-playground.exe", path.join(binDir, "liquid-verre-playground.exe"));
  push("bin/liquid-verre-playground", path.join(binDir, "liquid-verre-playground"));
  push("README.md", path.join(REPO_ROOT, "README.md"));
  push("LICENSE", path.join(REPO_ROOT, "LICENSE"));
  push("CHANGELOG.md", path.join(REPO_ROOT, "CHANGELOG.md"));
  // 着色器源：库运行期用 include_str! 内嵌，附带只为便于上层排查。
  const shaders = path.join(REPO_ROOT, "crates", "liquid-verre", "shaders");
  if (fs.existsSync(shaders)) {
    for (const f of fs.readdirSync(shaders).sort()) {
      if (f.endsWith(".hlsl")) push(`shaders/${f}`, path.join(shaders, f));
    }
  }
  return out;
}

const artifacts = collect();
if (!artifacts.length) {
  console.error("[package] 没有找到任何可打包产物，请先构建");
  process.exit(1);
}
for (const { rel, abs } of artifacts) {
  const dst = path.join(stage, rel);
  fs.mkdirSync(path.dirname(dst), { recursive: true });
  fs.copyFileSync(abs, dst);
}

const git = gitInfo();
const info = {
  name: "liquid-verre-rs",
  version,
  target: triple,
  profile,
  gitSha: git.sha,
  gitDirty: git.dirty,
  builtAtUtc: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
  node: process.version,
  artifacts: artifacts.map(({ rel, abs }) => ({ path: rel, bytes: fs.statSync(abs).size })),
};
fs.writeFileSync(path.join(stage, "build-info.json"), `${JSON.stringify(info, null, 2)}\n`);

console.log(`\n[package] 打包目录: ${stage}`);
for (const a of info.artifacts) console.log(`  ${a.path}  (${a.bytes} B)`);
if (noZip) process.exit(0);

const zipPath = path.join(outRoot, `liquid-verre-rs-${version}-${triple}.zip`);
const z = createZip(stage, zipPath);
console.log(`[package] 压缩包: ${zipPath}  (${z.files} 个文件, ${z.bytes} 字节)`);
