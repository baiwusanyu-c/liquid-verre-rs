#!/usr/bin/env node
// scripts/release.mjs —— 本地发版助手（GitHub 上的实际构建/发布由 .github/workflows/release.yml 完成）。
//
//   node scripts/release.mjs --bump patch --dry-run          # 只看会做什么（不落盘、不动 git）
//   node scripts/release.mjs --bump patch                    # 改版本号 + 改 CHANGELOG 标题 + 闸门 + package 干跑
//   node scripts/release.mjs --bump patch --ship             # 一键发版：上面全部 + commit + tag + push
//   node scripts/release.mjs --bump patch --ship --no-push   # 只 commit + tag，push 留给人复核
//   node scripts/release.mjs --version 0.5.0 --ship
//   node scripts/release.mjs --publish                       # 本地 cargo publish（需已 cargo login），不碰 git
//
// 不带 --ship 时**刻意不**做 git commit / tag / push：项目约定「提交必须由人显式发起」，脚本只把
// 「版本号、CHANGELOG 标题、本地质量闸门、cargo package 干跑」准备好，再打印接下来要敲的 git 命令。
// --ship 就是那句显式授权：add + commit（`release: x.y.z`）+ 附注 tag（`vx.y.z`）+ push 分支与 tag。
// tag 一推上去 release.yml 立刻跑：windows 上 check+package → ubuntu 上建 GitHub Release
// →（配了 secret CARGO_REGISTRY_TOKEN 才）cargo publish。
//
// 顺序上的两个取舍：
// - git 预检放在闸门**之前**：tag 重名 / detached HEAD / 落后上游这类错误没必要等 5 分钟 clippy 才发现。
// - 本地 --publish 放在 push **之前**：publish 失败时 tag 还没出去，回滚只需要动本地（脚本会打印回滚命令）。

import fs from "node:fs";
import path from "node:path";
import { REPO_ROOT, run, crateVersion, capture } from "./_lib.mjs";

const args = process.argv.slice(2);
const dry = args.includes("--dry-run");
const doPublish = args.includes("--publish");
const ship = args.includes("--ship");
const noPush = args.includes("--no-push");

function fail(msg) {
  console.error(`[release] ${msg}`);
  process.exit(1);
}

/** 取一个带值的旗标；给了旗标却没给值就失败（比静默回落到默认值好排查）。 */
function opt(name) {
  const i = args.indexOf(name);
  if (i < 0) return null;
  const v = args[i + 1];
  if (v === undefined || v.startsWith("--")) fail(`${name} 需要一个值`);
  return v;
}

/** git 子命令，非 0 退出即终止。 */
function git(...a) {
  if (run("git", a) !== 0) fail(`git ${a.join(" ")} 失败（exit 非 0）`);
}

/** 把 origin 的 URL 翻成网页地址，只为在最后打印 Actions 链接（ssh / https 两种写法都要认）。 */
function repoWebUrl(url) {
  if (!url) return null;
  let m = url.match(/^git@([^:]+):(.+?)(?:\.git)?$/);
  if (m) return `https://${m[1]}/${m[2]}`;
  m = url.match(/^(?:https?|ssh):\/\/(?:[^@/]+@)?(.+?)(?:\.git)?$/);
  if (m) return `https://${m[1]}`;
  return null;
}

if (noPush && !ship) fail("--no-push 只在 --ship 下有意义（不带 --ship 本来就不推送）");

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
const bump = opt("--bump");
let target = opt("--version");
if (target === null) {
  if (bump === null) fail("需要 --bump patch|minor|major 或 --version x.y.z");
  const parts = current.split(".").map((x) => parseInt(x, 10));
  if (parts.length !== 3 || parts.some((n) => !Number.isInteger(n))) {
    fail(`当前版本号 ${current} 不是 x.y.z，无法 --bump，请用 --version 指定`);
  }
  const [ma, mi, pa] = parts;
  if (bump === "major") target = `${ma + 1}.0.0`;
  else if (bump === "minor") target = `${ma}.${mi + 1}.0`;
  else if (bump === "patch") target = `${ma}.${mi}.${pa + 1}`;
  else fail(`未知 --bump ${bump}（用 patch|minor|major）`);
}
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(target)) fail(`非法版本号 ${target}`);
const tag = opt("--tag") ?? `v${target}`;
// release.yml 的触发条件是 tags: ["v*"]；tag 名不匹配就等于「发了个不会触发流水线的版本」。
if (!/^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(tag)) {
  fail(`tag 名 ${tag} 不匹配 release.yml 的触发条件（必须 v + semver，例 v${target}）`);
}
const commitMsg = opt("--message") ?? `release: ${target}`;

// ---- 3. git 预检（尽早失败，别浪费一次完整闸门）----
const branch = capture("git", ["rev-parse", "--abbrev-ref", "HEAD"]);
const origin = capture("git", ["remote", "get-url", "origin"]);
const willPush = ship && !noPush;
if (ship) {
  if (!branch || branch === "HEAD") fail("当前是 detached HEAD，先切回分支再 --ship");
  if (capture("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`])) {
    fail(`本地已存在 tag ${tag}（这一版可能发过了）。若只是上次 push 失败：git push origin ${branch} ${tag}`);
  }
  if (willPush) {
    if (!origin) fail("没有配置 origin 远端，无法 push（改用 --ship --no-push 只提交打 tag）");
    const remoteTag = capture("git", ["ls-remote", "--tags", "origin", tag]);
    if (remoteTag && remoteTag.trim().length) fail(`远端已存在 tag ${tag}（这一版可能发过了）`);
    // 没有上游（返回 null）时跳过：push 自己会把分支建出来。
    const behind = capture("git", ["rev-list", "--count", "HEAD..@{u}"]);
    if (behind !== null && parseInt(behind, 10) > 0) {
      fail(`本地落后上游 ${behind} 个提交，先 git pull --rebase 再发版（否则 push 会被拒）`);
    }
  }
}
// ---- 3.5 落盘内容预检（同样放在闸门之前：CHANGELOG 没维护就别白跑一次 clippy+test）----
const tomlPath = path.join(REPO_ROOT, "Cargo.toml");
const changelogPath = path.join(REPO_ROOT, "CHANGELOG.md");
const toml = fs.readFileSync(tomlPath, "utf8");
const changelog = fs.readFileSync(changelogPath, "utf8");
const versionRe = /^version = "[^"]+"/m;
if (!versionRe.test(toml)) fail("Cargo.toml 里没找到 [workspace.package] 的 version 行");
const newToml = toml.replace(versionRe, `version = "${target}"`);
const today = new Date().toISOString().slice(0, 10);
const newChangelog = changelog.replace(
  /^## \[[0-9][^ \]]*-unreleased\][^\n]*/m,
  `## [${target}] - ${today}`,
);
const changelogHit = newChangelog !== changelog;
// --ship 时 CHANGELOG 的这一段就是 release notes（release.yml 用 awk 抽 `## [x.y.z]`）；
// 没有 unreleased 标题 => notes 会是空的，这种版本不该一键发出去。
if (ship && !changelogHit) {
  fail(`CHANGELOG.md 里没有 \`## [x.y.z-unreleased]\` 标题可替换成 ${target}，--ship 会让 release notes 变空；先补 CHANGELOG，或去掉 --ship`);
}

// ---- 4. 本地质量闸门 ----
if (run("node", [path.join("scripts", "check.mjs")]) !== 0) fail("check 未通过，先修再发");

// ---- 5. crate 可发布性干跑 ----
// --allow-dirty：本地发版前工作区通常还没 commit；CI 上的 checkout 是干净的，
// 不需要这个旗标（见 .github/workflows/ci.yml）。
if (run("cargo", ["package", "--package", "liquid-verre-rs", "--list", "--allow-dirty"]) !== 0) {
  fail("cargo package --list 失败（readme/license/路径依赖等问题）");
}

// ---- 6. 落盘：版本号 + CHANGELOG 标题（内容已在 3.5 算好并校验过）----
fs.writeFileSync(tomlPath, newToml);
if (changelogHit) fs.writeFileSync(changelogPath, newChangelog);
console.log(`[release] 版本 ${current} -> ${target}${changelogHit ? "，CHANGELOG 标题已替换" : "（CHANGELOG 未改动）"}`);

// ---- 7. 可选：commit + tag（+ push）----
if (ship) {
  const pending = capture("git", ["status", "--porcelain"]) ?? "";
  if (pending.trim().length) {
    git("add", "-A");
    git("commit", "-m", commitMsg);
    console.log(`[release] 已提交：${commitMsg}`);
  } else {
    console.log("[release] 没有需要提交的改动（版本号与 CHANGELOG 已是目标状态）");
  }
  git("tag", "-a", tag, "-m", `${tag} — liquid-verre-rs ${target}`);
  console.log(`[release] 已打附注 tag ${tag}`);
}

// ---- 8. 可选：本地直接发布到 crates.io（放在 push 之前，失败时 tag 还没出去）----
if (doPublish) {
  if (willPush) {
    console.warn("[release] 注意：本地 publish 之后又推 tag，若仓库同时配了 secret CARGO_REGISTRY_TOKEN，");
    console.warn("          CI 的 crates-io job 会因「该版本已存在」失败（GitHub Release 不受影响）。二选一即可。");
  }
  if (run("cargo", ["publish", "--package", "liquid-verre-rs", "--no-verify"]) !== 0) {
    if (ship) {
      console.error(`[release] 回滚本地提交与 tag：git tag -d ${tag} && git reset --hard HEAD~1`);
    }
    fail("cargo publish 失败（是否已 cargo login / 配好 CARGO_REGISTRY_TOKEN？）");
  }
}

// ---- 9. 推送：分支 + tag 一次推完，tag 落地即触发 release.yml ----
const web = repoWebUrl(origin);
if (willPush) {
  git("push", "origin", `HEAD:refs/heads/${branch}`, `refs/tags/${tag}`);
  console.log(`\n[release] ${target} 已推送（分支 ${branch} + tag ${tag}），发布流水线已触发：`);
  console.log(`  ${web ? `${web}/actions` : "GitHub → Actions → release"}`);
  console.log("  1) windows-latest：check + package（dist/*.zip + build-info.json）");
  console.log("  2) ubuntu-latest：从 CHANGELOG 抽本版本段落建 GitHub Release");
  console.log("  3) ubuntu-latest：配了 secret CARGO_REGISTRY_TOKEN 才 cargo publish，没配则跳过并留一条 notice");
  if (web) console.log(`  Release 页面：${web}/releases/tag/${tag}`);
} else if (ship) {
  console.log(`\n[release] ${target} 已提交并打好 tag（--no-push）。复核后推送：`);
  console.log(`  git push origin ${branch} ${tag}`);
} else {
  console.log(`
[release] 接下来由你执行（不带 --ship 时脚本刻意不 commit/tag/push）：
  git add -A && git commit -m "${commitMsg}"
  git tag -a ${tag} -m "${tag}"
  git push origin ${branch ?? "HEAD"} ${tag}
推送 tag 后 .github/workflows/release.yml 会自动构建、建 GitHub Release，
并在配置了 secret CARGO_REGISTRY_TOKEN 时发布到 crates.io。
`);
}