//! `liquid-verre-rs` 的构建脚本：把构建期元数据固化成 `OUT_DIR/build_info.rs`。
//!
//! # 为什么需要构建期元数据
//!
//! 库要在运行期报告「我是哪个版本、构建自哪个 commit、内嵌了哪些着色器」。这些信息
//! 只在构建那一刻可得（git 状态、目标三元组、profile、着色器内容摘要），所以由本脚本
//! 采集，生成一段可直接编译的 Rust 片段；`src/build_info.rs` 用 `include!` 原样纳入。
//!
//! # 契约（改动必须同步 `src/build_info.rs`）
//!
//! 生成文件里的条目，**名字 / 类型 / 顺序**都属于公共契约，一个都不能改：
//!
//! ```text
//! pub const VERSION: &str              // CARGO_PKG_VERSION
//! pub const GIT_SHA: &str              // git 短 sha（12 位），取不到为 "unknown"
//! pub const GIT_DIRTY: bool            // 工作区是否有未提交改动
//! pub const BUILD_TIMESTAMP_UTC: &str  // 形如 2026-10-20T05:00:00Z
//! pub const TARGET_TRIPLE: &str        // TARGET 环境变量
//! pub const RUSTC_VERSION: &str        // `rustc --version`，取不到为 "unknown"
//! pub const PROFILE: &str              // PROFILE 环境变量（debug / release）
//! pub const SHADERS: &[(&str, &str)]   // (文件名, sha256 前 12 位)，按文件名升序
//! ```
//!
//! # 硬约束 1：零第三方依赖
//!
//! 本文件不引任何 crate（`[build-dependencies]` 保持为空），SHA-256 与 UTC 时间戳都自己实现：
//!
//! - build script 编译失败会阻塞**整个** workspace，依赖越少越稳；
//! - 离线 / CI 冷缓存 / 内网镜像不可达时依然能构建；
//! - 这两段都只是几十行整数运算，为此拉 `sha2` + `chrono` 不划算。
//!
//! # 硬约束 2：build.rs 不许失败
//!
//! 拿不到 git 是**正常情况**（发布 tarball 里没有 `.git`、shallow clone、离线、无 git 环境），
//! 一律降级成 `"unknown"` / `false`，绝不 `panic!`——否则库在别人机器上直接编不过。
//! 唯一的例外是 [`self_check`]：那是本文件自身实现被改坏的情况，宁可构建失败，
//! 也不能把错的摘要写进产物元数据。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// SHA-256 的 64 个轮常量：第 i 项 = `floor(2^32 * frac(cbrt(第 i 个素数)))`。
/// 标准写死的表，照抄即可；写成常量而不是运行时算，是为了能和规范逐字核对。
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// 十六进制小写字母表。手写查表而不是 `format!("{:02x}")`：少一层格式化状态机。
const HEX: &[u8; 16] = b"0123456789abcdef";

/// git / rustc 拿不到信息时统一填的占位值。
const UNKNOWN: &str = "unknown";

fn main() {
    // 自检必须最先跑：SHA-256 或日期换算被改坏时，绝不能把错的元数据写进 OUT_DIR。
    self_check();

    // 这两个变量由 cargo 注入；万一在 cargo 之外被直接执行，退化成当前目录而不是崩溃。
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let out_dir = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let shaders = collect_shaders(&manifest_dir.join("shaders"));
    emit_rerun_triggers(&shaders);

    let info = BuildInfo {
        version: env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| UNKNOWN.to_string()),
        git_sha: git_short_sha(&manifest_dir),
        git_dirty: git_worktree_dirty(&manifest_dir),
        built_at: timestamp_utc(),
        target: env::var("TARGET").unwrap_or_else(|_| UNKNOWN.to_string()),
        rustc: rustc_version(),
        profile: env::var("PROFILE").unwrap_or_else(|_| UNKNOWN.to_string()),
        shaders: shader_digests(&shaders),
    };

    let target_path = out_dir.join("build_info.rs");
    if let Err(e) = fs::write(&target_path, render(&info)) {
        // 写不进 OUT_DIR 意味着 cargo 的构建目录已经出问题了，继续下去只会得到更晚更难懂的报错。
        panic!("无法写出 {}: {e}", target_path.display());
    }
}

/// 告诉 cargo「只有这些东西变了才重跑构建脚本」。
///
/// 不显式声明的话，cargo 默认监视整个 package 目录：任何文件改动都会重跑 build script，
/// 连带让 `BUILD_TIMESTAMP_UTC` 每次都变——既拖慢增量构建，也让元数据失去可比性
/// （性能回归排查时「同一个 commit 的两次构建」必须是同一份 build_info）。
fn emit_rerun_triggers(shaders: &[(String, PathBuf)]) {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=shaders");
    // 目录 mtime 在部分文件系统上不可靠（新增/删除文件未必更新父目录），所以逐个文件再报一遍。
    for (name, _) in shaders {
        println!("cargo:rerun-if-changed=shaders/{name}");
    }
    println!("cargo:rerun-if-changed=Cargo.toml");
    // workspace 根的 Cargo.toml 决定 version / profile / rust-version，改了也要重算元数据。
    println!("cargo:rerun-if-changed=../../Cargo.toml");
}

/// 列出 `shaders/*.hlsl`，返回按文件名升序的 `(文件名, 路径)`。
///
/// 目录不存在（例如发布 tarball 里裁掉了着色器）时返回空表——构建照常成功，
/// 只是 `SHADERS` 变成空切片，运行期由 `include_str!` 的缺失自行报错。
fn collect_shaders(shaders_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let Ok(entries) = fs::read_dir(shaders_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_hlsl = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("hlsl"))
            .unwrap_or(false);
        if !is_hlsl {
            continue;
        }
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        out.push((name, path));
    }
    // 顺序即契约：SHADERS 必须按文件名升序，否则摘要列表在不同文件系统上会漂移。
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 计算每个着色器的 SHA-256，取前 12 位十六进制。
///
/// 只取 12 位是刻意的：这一串会进日志、进 `build-info.json`、进排障截图，
/// 全长 64 位没有额外价值（我们只需要「着色器换没换」，不是抗碰撞签名）。
fn shader_digests(shaders: &[(String, PathBuf)]) -> Vec<(String, String)> {
    shaders
        .iter()
        .map(|(name, path)| {
            // 列得出目录却读不出文件，在 Windows 上基本只可能是权限/杀软拦截；
            // 用 12 个 0 占位以保持「12 位十六进制」的形状，不让构建失败。
            let digest = match fs::read(path) {
                Ok(bytes) => hex(&sha256(&bytes))[..12].to_string(),
                Err(_) => "0".repeat(12),
            };
            (name.clone(), digest)
        })
        .collect()
}
// ---------------------------------------------------------------- SHA-256（纯 std 实现）

/// 最小可用的 SHA-256（FIPS 180-4）。
///
/// 只服务于 `SHADERS` 摘要这一个用途，所以刻意做成最朴素的单线程、无增量 API 版本：
/// 输入一次性喂进来，输出一段 32 字节摘要。输入最长就是一个 `.hlsl` 源文件（几十 KB），
/// 全量拷进 `Vec` 做 padding 的开销可以忽略。
fn sha256(data: &[u8]) -> [u8; 32] {
    // H0：前 8 个素数平方根小数部分的前 32 位。
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // 预处理：追加 0x80，补 0x00 直到「总长 % 64 == 56」，最后 8 字节放原始比特长度（大端）。
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg: Vec<u8> = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 64];
    for block in msg.chunks_exact(64) {
        // 消息调度：前 16 个字直接取大端，后 48 个由 σ0 / σ1 递推。
        for (i, word) in w[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        // 每个 block 结束把压缩结果累加回 H（Davies–Meyer 结构）。
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// 字节串 → 小写十六进制字符串。
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(HEX[(b >> 4) as usize]));
        s.push(char::from(HEX[(b & 0x0f) as usize]));
    }
    s
}

/// 构建期自检：SHA-256 已知向量 + civil-from-days 已知日期。
///
/// 为什么用 `assert_eq!`（即 `panic!`）而不是 `#[cfg(test)]` 单测：
/// **build.rs 不是 cargo 的测试目标**。`cargo test` 只编译 `src/` 下声明的 target，
/// 构建脚本永远以 `cfg(test)` 为假编译，`#[cfg(test)] mod tests` 写在这里一行都跑不到。
/// 所以自检只能挂在 `fn main()` 开头，每次构建都跑（成本远小于 1 ms）：
/// 一旦有人改坏了 SHA-256 或日期换算，立刻在构建阶段炸掉，
/// 而不是把错的摘要悄悄写进 `build_info.rs`，再顺着流进发布产物和排障日志。
fn self_check() {
    // 标准向量：空输入 / "abc" / 56 字节（padding 正好被挤进下一个 block，专门覆盖边界）。
    let vectors: &[(&[u8], &str)] = &[
        (
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];
    for (input, want) in vectors {
        let got = hex(&sha256(input));
        assert_eq!(
            got.as_str(),
            *want,
            "SHA-256 自检失败：build.rs 里的实现被改坏了"
        );
    }

    // 日期向量：Unix 纪元 / 纪元前一天 / 2000 闰日（三百年规则）/ 普通日期。
    let dates: &[(i64, (i64, u32, u32))] = &[
        (0, (1970, 1, 1)),
        (-1, (1969, 12, 31)),
        (11_016, (2000, 2, 29)),
        (20_000, (2024, 10, 4)),
    ];
    for &(days, want) in dates {
        assert_eq!(
            civil_from_days(days),
            want,
            "civil-from-days 自检失败：BUILD_TIMESTAMP_UTC 会是错的"
        );
    }
}

// ---------------------------------------------------------------- 构建环境采集

/// `git rev-parse --short=12 HEAD`；任何异常都降级为 `"unknown"`。
///
/// 这里不检查 `.git` 是否存在就直接调 git：worktree / submodule / tarball 的目录形态太多，
/// 让 git 自己判断比我们自己猜更准，失败就走降级路径。
fn git_short_sha(repo_dir: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        // 空输出 / 离谱长度（例如 git 把错误信息写进了 stdout）都不要。
        .filter(|s| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| UNKNOWN.to_string())
}

/// 工作区是否有未提交改动（`git status --porcelain` 有任意输出即为 dirty）。
///
/// git 不可用时返回 `false` 而不是 `true`：这里的语义是「构建可信度标记」，
/// 在无法判定的环境下把每个产物都打成 dirty 只会让下游忽略这个字段。
fn git_worktree_dirty(repo_dir: &Path) -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.iter().all(u8::is_ascii_whitespace))
        .unwrap_or(false)
}

/// `$RUSTC --version` 的第一行；失败则为 `"unknown"`。
///
/// 用 cargo 注入的 `RUSTC` 而不是裸 `rustc`：rustup 多工具链 / CI 里指定 toolchain 时，
/// 只有 `RUSTC` 才指向真正编译这个 crate 的那一个 rustc。
fn rustc_version() -> String {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| UNKNOWN.to_string())
}
// ---------------------------------------------------------------- UTC 时间戳

/// 当前 UTC 时间，格式 `2026-10-20T05:00:00Z`。
///
/// 不引 chrono，也不 shell 出去调 PowerShell：构建期每多一个外部进程就多一个失败面
/// （CI 容器里没有 powershell、Linux/macOS 上交叉编译、受限环境禁止起子进程）。
/// 「从 Unix 秒算民用日期」本身就是一段整数运算，自己写反而更稳、更快。
fn timestamp_utc() -> String {
    // 系统时钟被设到 1970 之前时 duration_since 会失败；退回纪元零点，而不是让构建崩掉。
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // div_euclid / rem_euclid：保证 1970 之前的时间也能落到正确的「天 + 当天秒数」。
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = secs_of_day % 3_600 / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// civil-from-days：把「距 1970-01-01 的天数」换算成 `(年, 月, 日)`。
///
/// 算法即 Howard Hinnant 的 `civil_from_days`（公有领域）：把纪元平移到「3 月 1 日为
/// 一年之始」的 400 年周期里，再用 146097 / 1461 / 365 逐级取商。全程只有整数除法与
/// 取模，不用闰年表也不用浮点。已在 [`self_check`] 里用四个向量（含 2000-02-29）校验。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    // 逐级取商得到「3 月起算」的年内偏移，各中间量的取值范围：
    //   doe   day of era             [0, 146096]
    //   yoe   year of era            [0, 399]
    //   doy   day of year            [0, 365]
    //   mp    月序，0 = 3 月          [0, 11]
    //   day                          [1, 31]
    //   month                        [1, 12]
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    // 3 月起算的年份：1、2 月属于下一个公历年，要补 1。
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

// ---------------------------------------------------------------- 生成 build_info.rs

/// 生成文件需要的全部字段。
///
/// 用结构体而不是 8 个函数参数：参数一多，调用点就看不出哪个值对应哪个常量，
/// 而这里的对应关系属于契约（见文件头文档），命名字段更安全。
struct BuildInfo {
    version: String,
    git_sha: String,
    git_dirty: bool,
    built_at: String,
    target: String,
    rustc: String,
    profile: String,
    shaders: Vec<(String, String)>,
}

/// 把任意字符串安全地转成 Rust 字符串字面量（含首尾双引号）。
///
/// git 输出、环境变量、文件名里都可能出现反斜杠与引号（Windows 路径尤其常见），
/// 不转义就会生成语法错误的 `build_info.rs`，把整个 workspace 的编译一起带崩。
fn lit(value: &str) -> String {
    let mut s = String::with_capacity(value.len() + 2);
    s.push('"');
    for c in value.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            // 其余控制字符统一走 \u{..}：生成文件必须是纯文本、可 diff、行结构稳定。
            c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                s.push_str(&format!("\\u{{{:04x}}}", u32::from(c)));
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

/// 渲染 `build_info.rs` 全文。
///
/// 条目的名字 / 类型 / 顺序 / 文档注释都是契约（见文件头），`src/build_info.rs` 依赖它们，
/// 增删或换序都会让库编译失败或让下游读到错位的元数据。
fn render(info: &BuildInfo) -> String {
    let mut out = String::with_capacity(2048);

    // 头部说明：include! 进来的代码不会带上 build.rs 的文档注释，
    // 不在生成文件里留一句话，后来者会以为这是个可以手改的普通源文件。
    out.push_str(
        "// 由 crates/liquid-verre/build.rs 在构建期自动生成，请勿手工编辑，也不要提交进 git。\n",
    );
    out.push_str("// 契约：下面每个条目的名字 / 类型 / 顺序都不可改；src/build_info.rs 用 include! 纳入本文件。\n");

    let scalars: [(&str, String); 7] = [
        (
            "/// 库版本号（来自 CARGO_PKG_VERSION）",
            format!("pub const VERSION: &str = {};", lit(&info.version)),
        ),
        (
            "/// 构建时的 git 短 sha（12 位）；取不到时为 \"unknown\"",
            format!("pub const GIT_SHA: &str = {};", lit(&info.git_sha)),
        ),
        (
            "/// 工作区是否有未提交改动",
            format!("pub const GIT_DIRTY: bool = {};", info.git_dirty),
        ),
        (
            "/// 构建时间（UTC，ISO8601，形如 2026-10-20T05:00:00Z）",
            format!(
                "pub const BUILD_TIMESTAMP_UTC: &str = {};",
                lit(&info.built_at)
            ),
        ),
        (
            "/// 目标三元组（来自 TARGET 环境变量）",
            format!("pub const TARGET_TRIPLE: &str = {};", lit(&info.target)),
        ),
        (
            "/// rustc 版本（来自 RUSTC 环境变量 + `--version`，失败则 \"unknown\"）",
            format!("pub const RUSTC_VERSION: &str = {};", lit(&info.rustc)),
        ),
        (
            "/// 构建 profile（来自 PROFILE 环境变量）",
            format!("pub const PROFILE: &str = {};", lit(&info.profile)),
        ),
    ];
    for (doc, decl) in scalars {
        out.push('\n');
        out.push_str(doc);
        out.push('\n');
        out.push_str(&decl);
        out.push('\n');
    }

    out.push('\n');
    out.push_str("/// 每个内嵌着色器的 (文件名, sha256 前 12 位十六进制)，按文件名升序\n");
    if info.shaders.is_empty() {
        // 空表也要生成合法表达式，`include!` 那一侧不需要为「没有着色器」写特例。
        out.push_str("pub const SHADERS: &[(&str, &str)] = &[];\n");
    } else {
        out.push_str("pub const SHADERS: &[(&str, &str)] = &[\n");
        for (name, digest) in &info.shaders {
            out.push_str(&format!("    ({}, {}),\n", lit(name), lit(digest)));
        }
        out.push_str("];\n");
    }
    out
}
