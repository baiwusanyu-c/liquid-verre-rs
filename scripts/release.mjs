#!/usr/bin/env node
// scripts/release.mjs —— 本地发版助手（GitHub 上的实际构建/发布由 .github/workflows/release.yml 完成）。
//
//   node scripts/release.mjs --bump patch --dry-run   # 只看会做什么
//   node scripts/release.mjs --bump patch             # 改版本 + 改 CHANGELOG 标题 + 跑 check + 提示后续命令
//   node scripts/release.mjs --version 0.5.0
//   node scripts/release.mjs --publish                # 额外执行 cargo publish（需要已 cargo login）
//
// 它**不**做 git commit / tag / push：按项目约定，提交必须由人显式发起。
// 它只把「版本号、CHANGELOG 标题、本地质量闸门、cargo package 干跑」准备好，
// 然后打印接下来要敲的三条 git 命令。

import fs from "node:fs";
import path from "node:path";
import { REPO_ROOT, run, crateVersion, capture } from "./_lib.mjs";

const args = process.argv.slice(2);
const dry = args.includes("--dry-run");
const doPublish = args.includes("--publish");
const bumpIdx = args.indexOf("--bump");
const verIdx = args.indexOf("--version");
const tagIdx = args.indexOf("--tag");

function fail(msg) {
  console.error(`[release] ${msg}`);
  process.exit(1);
}

// ---- 1. 工作区必须干净（否则发出去的版本对不上 commit）----
const dirty = capture("git", ["status", "--porcelain"]);
if (dirty === null) fail("git 不可用");
if (dirty.trim().length) {
  console.error("[release] 工作区有未提交改动，先处理再发版：");
  console.error(dirty.split("\n").slice(0, 20).map((l) => "  " + l).join("\n"));
  fail("dirty tree");
}

// ---- 2. 目标版本 ----
const current = crateVersion();
let target = null;
if (verIdx >= 0) target = args[verIdx + 1];
else if (bumpIdx >= 0) {
  const [ma, mi, pa] = current.split(".").map((x) => parseInt(x, 10));
  const kind = args[bumpIdx + 1];
  if (kind === "major") target = `${ma + 1}.0.0`;
  else if (kind === "minor") target = `${ma}.${mi + 1}.0`;
  else if (kind === "patch") target = `${ma}.${mi}.${pa + 1}`;
  else fail(`未知 --bump ${kind}（用 patch|minor|major）`);
} else fail("需要 --bump patch|minor|major 或 --version x.y.z");
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(target)) fail(`非法版本号 ${target}`);
const tag = tagIdx >= 0 ? args[tagIdx + 1] : `v${target}`;

// ---- 3. 本地质量闸门 ----
if (run("node", [path.join("scripts", "check.mjs")]) !== 0) fail("check 未通过，先修再发");

// ---- 4. crate 可发布性干跑 ----
// --allow-dirty：本地发版前工作区通常还没 commit；CI 上的 checkout 是干净的，
// 不需要这个旗标（见 .github/workflows/ci.yml）。
if (run("cargo", ["package", "--package", "liquid-verre-rs", "--list", "--allow-dirty"]) !== 0) {
  fail("cargo package --list 失败（readme/license/路径依赖等问题）");
}

// ---- 5. 落盘：版本号 + CHANGELOG 标题 ----
const tomlPath = path.join(REPO_ROOT, "Cargo.toml");
const changelogPath = path.join(REPO_ROOT, "CHANGELOG.md");
const toml = fs.readFileSync(tomlPath, "utf8");
const changelog = fs.readFileSync(changelogPath, "utf8");
const newToml = toml.replace(/^version = "[^"]+"/m, `version = "${target}"`);
const today = new Date().toISOString().slice(0, 10);
const newChangelog = changelog.replace(
  /^## \[[0-9][^ \]]*-unreleased\][^\n]*/m,
  `## [${target}] - ${today}`,
);
if (dry) {
  console.log(`[release][dry-run] 版本 ${current} -> ${target}（tag ${tag}）`);
  console.log("[release][dry-run] 将修改: Cargo.toml 的 version、CHANGELOG.md 的 unreleased 标题");
  if (newChangelog === changelog) console.log("[release][dry-run] 警告: CHANGELOG 里没有 *-unreleased 标题可替换");
  process.exit(0);
}
if (newToml === toml) fail("Cargo.toml 里没找到 version 行");
fs.writeFileSync(tomlPath, newToml);
fs.writeFileSync(changelogPath, newChangelog === changelog ? changelog : newChangelog);
console.log(`[release] 版本 ${current} -> ${target}，CHANGELOG 标题已替换`);

// ---- 6. 可选：直接发布到 crates.io ----
if (doPublish) {
  if (run("cargo", ["publish", "--package", "liquid-verre-rs", "--no-verify"]) !== 0) {
    fail("cargo publish 失败（是否已 cargo login？）");
  }
}

console.log(`
[release] 接下来由你执行（本脚本刻意不 commit/tag/push）：
  git add -A && git commit -m "release: ${target}"
  git tag ${tag}
  git push origin HEAD --tags
推送 tag 后 .github/workflows/release.yml 会自动：
  1) windows 上跑 check + package；
  2) 建 GitHub Release 并挂 dist zip + build-info.json；
  3) 若配置了 secret CARGO_REGISTRY_TOKEN，则 cargo publish 到 crates.io。
`);
