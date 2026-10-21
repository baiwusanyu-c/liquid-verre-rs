// scripts/_lib.mjs —— 构建脚本共用工具（零第三方依赖，只用 Node 内置模块）。
//
// 为什么是 Node 而不是 PowerShell：.ps1 只在 Windows 上跑，而且 PS 5.1 读无 BOM 的
// UTF-8 会把中文注释变成乱码、[CmdletBinding()] 还会和自己的 -Debug 参数撞名。
// Node 脚本跨平台、编码固定 UTF-8、退出码透传干净。
//
// 约定：所有脚本都不引入 npm 依赖，`node scripts/xxx.mjs` 直接可跑。

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs";
import zlib from "node:zlib";

/** 仓库根目录（= 本文件所在目录的上一级）。 */
export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/**
 * 运行一个命令，stdio 直通，返回退出码。
 * Windows 上 `cargo` 是 cargo.exe，Node 的 CreateProcess 能直接找到；
 * 万一 ENOENT（例如通过 .cmd 垫片安装的工具链），自动补 `.exe` 再试一次。
 */
export function run(cmd, args, opts = {}) {
  const options = { stdio: "inherit", cwd: opts.cwd ?? REPO_ROOT, env: process.env };
  console.log(`$ ${cmd} ${args.join(" ")}`);
  let r = spawnSync(cmd, args, options);
  if (r.error && r.error.code === "ENOENT" && process.platform === "win32" && !cmd.endsWith(".exe")) {
    r = spawnSync(`${cmd}.exe`, args, options);
  }
  if (r.error) {
    console.error(`[scripts] 无法启动 ${cmd}: ${r.error.message}`);
    return 1;
  }
  if (r.signal) {
    console.error(`[scripts] ${cmd} 被信号 ${r.signal} 终止`);
    return 1;
  }
  return r.status ?? 1;
}

/** 运行命令并捕获 stdout（失败返回 null）。 */
export function capture(cmd, args, cwd = REPO_ROOT) {
  const r = spawnSync(cmd, args, { cwd, encoding: "utf8" });
  if (r.error || r.status !== 0) return null;
  return (r.stdout ?? "").trim();
}

/** Python 解释器：优先 PYTHON 环境变量，其次 python3，最后 python。 */
export function pythonExe() {
  if (process.env.PYTHON) return process.env.PYTHON;
  const probe = spawnSync("python3", ["--version"], { encoding: "utf8" });
  return probe.error || probe.status !== 0 ? "python" : "python3";
}

/** cargo build 的 profile 目录名。 */
export function profileDir(release = true) {
  return release ? "release" : "debug";
}

/** 读 crate 版本（根 Cargo.toml 的 [workspace.package] version）。 */
export function crateVersion() {
  const toml = fs.readFileSync(path.join(REPO_ROOT, "Cargo.toml"), "utf8");
  const m = toml.match(/^\s*version\s*=\s*"([^"]+)"/m);
  return m ? m[1] : "0.0.0";
}

/** 目标三元组：读 `rustc -vV` 的 host 行（不要硬编码，本机可能是 gnu 也可能是 msvc）。 */
export function targetTriple() {
  const out = capture("rustc", ["-vV"]);
  const m = out?.match(/^host:\s*(\S+)$/m);
  return m ? m[1] : "unknown-target";
}

/** git 短 sha 与工作区是否脏。 */
export function gitInfo() {
  const sha = capture("git", ["rev-parse", "--short=12", "HEAD"]) ?? "unknown";
  const status = capture("git", ["status", "--porcelain"]) ?? "";
  return { sha, dirty: status.length > 0 };
}

function dosDateTime(date = new Date()) {
  const time = (date.getHours() << 11) | (date.getMinutes() << 5) | (date.getSeconds() >> 1);
  const day =
    ((date.getFullYear() - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate();
  return { time, day };
}

/**
 * 递归收集目录下的文件，返回相对路径（用 `/` 分隔，zip 规范要求）。
 */
export function walk(dir, base = dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, base, out);
    else if (entry.isFile()) out.push(path.relative(base, full).split(path.sep).join("/"));
  }
  return out.sort();
}

/**
 * 生成一个 zip（deflate 压缩），零依赖。
 *
 * 不用 PowerShell 的 Compress-Archive（跨平台差、且要求 UTF-8 BOM 脚本），
 * 也不用 bsdtar（Windows 10 以前的镜像里可能没有）。
 * 只实现 zip 里我们用得到的那部分：deflate(8) + UTF-8 文件名标志位(bit 11)。
 */
export function createZip(sourceDir, outZip) {
  const names = walk(sourceDir);
  const { time, day } = dosDateTime();
  const locals = [];
  const centrals = [];
  let offset = 0;

  for (const name of names) {
    const data = fs.readFileSync(path.join(sourceDir, name));
    const crc = zlib.crc32(data) >>> 0;
    const deflated = zlib.deflateRawSync(data, { level: 9 });
    // 压不动（已经压过的产物，例如 .rlib/.exe）就存原始数据，method=0。
    const useDeflate = deflated.length < data.length;
    const payload = useDeflate ? deflated : data;
    const method = useDeflate ? 8 : 0;
    const nameBuf = Buffer.from(name, "utf8");
    const ascii = /^[\x20-\x7e]*$/.test(name);
    const flags = ascii ? 0 : 0x0800; // bit 11 = 文件名是 UTF-8

    const lh = Buffer.alloc(30);
    lh.writeUInt32LE(0x04034b50, 0);
    lh.writeUInt16LE(20, 4); // version needed
    lh.writeUInt16LE(flags, 6);
    lh.writeUInt16LE(method, 8);
    lh.writeUInt16LE(time, 10);
    lh.writeUInt16LE(day, 12);
    lh.writeUInt32LE(crc, 14);
    lh.writeUInt32LE(payload.length, 18);
    lh.writeUInt32LE(data.length, 22);
    lh.writeUInt16LE(nameBuf.length, 26);
    lh.writeUInt16LE(0, 28);
    locals.push(lh, nameBuf, payload);

    const ch = Buffer.alloc(46);
    ch.writeUInt32LE(0x02014b50, 0);
    ch.writeUInt16LE(20, 4); // version made by
    ch.writeUInt16LE(20, 6); // version needed
    ch.writeUInt16LE(flags, 8);
    ch.writeUInt16LE(method, 10);
    ch.writeUInt16LE(time, 12);
    ch.writeUInt16LE(day, 14);
    ch.writeUInt32LE(crc, 16);
    ch.writeUInt32LE(payload.length, 20);
    ch.writeUInt32LE(data.length, 24);
    ch.writeUInt16LE(nameBuf.length, 28);
    // 30..46: extra/comment/disk/attrs 全 0
    ch.writeUInt32LE(offset, 42);
    centrals.push(ch, nameBuf);

    offset += lh.length + nameBuf.length + payload.length;
  }

  const localBuf = Buffer.concat(locals);
  const centralBuf = Buffer.concat(centrals);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(names.length, 8);
  eocd.writeUInt16LE(names.length, 10);
  eocd.writeUInt32LE(centralBuf.length, 12);
  eocd.writeUInt32LE(localBuf.length, 16);

  fs.mkdirSync(path.dirname(outZip), { recursive: true });
  fs.writeFileSync(outZip, Buffer.concat([localBuf, centralBuf, eocd]));
  return { files: names.length, bytes: fs.statSync(outZip).size };
}
