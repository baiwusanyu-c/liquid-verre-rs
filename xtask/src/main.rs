//! `cargo xtask <cmd>` —— liquid-verre-rs 的构建 / 检查 / 打包 / 性能阀门 runner。
//!
//! 设计原则：
//! - **零第三方依赖**（纯 std），避免构建工具本身把依赖图撑大；
//! - 只做「一条命令跑完一件事」的编排，真正的判定逻辑留在
//!   `tools/gate_perf.py`（性能阀门）与 `cargo test`（功能阀门）里，避免两处阈值漂移；
//! - 所有产物落到仓库根的 `dist/`，打包不涉及发版 / 部署（按需求只做「打包构建脚本」）。
//!
//! 用法：
//! ```text
//! cargo xtask build [--debug]              # 构建 workspace（默认 release）
//! cargo xtask check                        # fmt --check + clippy -D warnings + test --release
//! cargo xtask package [--out dist] [--no-zip]
//! cargo xtask gate [-- seconds]            # 调 tools/gate_perf.py 跑性能硬门槛
//! cargo xtask clean                        # cargo clean + 删 dist/
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// workspace 根目录 = xtask 可执行文件所在目录向上找到含 `Cargo.toml` + `crates/` 的那一层。
fn workspace_root() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("无法获取当前目录"));
    // xtask 在 <root>/xtask，因此父目录即根；若被从别处调用则逐级向上找。
    let mut dir = manifest.as_path();
    if dir.file_name().map(|n| n == "xtask").unwrap_or(false) {
        if let Some(parent) = dir.parent() {
            dir = parent;
        }
    }
    dir.to_path_buf()
}

fn cargo() -> Command {
    let exe = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    Command::new(exe)
}

fn run(cmd: &mut Command) -> Result<(), String> {
    println!("$ {:?}", cmd);
    let status = cmd
        .status()
        .map_err(|e| format!("无法启动 {:?}: {e}", cmd.get_program()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("命令失败（退出码 {:?}）", status.code()))
    }
}

fn python() -> PathBuf {
    // Windows 上 `python3` 常常不存在；`python` 是官方安装器的默认名字。
    PathBuf::from(std::env::var("PYTHON").unwrap_or_else(|_| "python".into()))
}

// ---------------------------------------------------------------- build

fn cmd_build(args: &[String]) -> Result<(), String> {
    let release = !args.iter().any(|a| a == "--debug");
    let mut c = cargo();
    c.arg("build").arg("--workspace");
    if release {
        c.arg("--release");
    }
    c.current_dir(workspace_root());
    run(&mut c)
}

// ---------------------------------------------------------------- check

fn cmd_check() -> Result<(), String> {
    let root = workspace_root();
    let mut steps: Vec<(&str, Command)> = Vec::new();

    let mut fmt = cargo();
    fmt.arg("fmt").arg("--all").arg("--").arg("--check");
    fmt.current_dir(&root);
    steps.push(("rustfmt", fmt));

    let mut clippy = cargo();
    clippy
        .arg("clippy")
        .arg("--workspace")
        .arg("--all-targets")
        .arg("--release")
        .arg("--")
        .arg("-D")
        .arg("warnings");
    clippy.current_dir(&root);
    steps.push(("clippy", clippy));

    let mut test = cargo();
    test.arg("test").arg("--workspace").arg("--release");
    test.current_dir(&root);
    steps.push(("test", test));

    let mut failed = Vec::new();
    for (name, mut cmd) in steps {
        println!("\n==== check: {name} ====");
        if let Err(e) = run(&mut cmd) {
            eprintln!("[xtask] {name} 失败: {e}");
            failed.push(name.to_string());
        }
    }
    if failed.is_empty() {
        println!("\n[xtask] check 全绿");
        Ok(())
    } else {
        Err(format!("check 未通过: {}", failed.join(", ")))
    }
}

// ---------------------------------------------------------------- package

/// 收集打包产物清单。返回 (相对 dist 的目标路径, 源文件绝对路径)。
fn collect_artifacts(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let rel = Path::new("target").join("release");
    let mut out: Vec<(PathBuf, PathBuf)> = Vec::new();

    // 库产物：rlib 一定有；cdylib/staticlib 只在开启对应 crate-type 时存在。
    for name in [
        "libliquid_verre.rlib",
        "liquid_verre.dll",
        "libliquid_verre.a",
    ] {
        let p = root.join(&rel).join(name);
        if p.exists() {
            out.push((PathBuf::from("lib").join(name), p));
        }
    }
    // dev playground（便于验证产物可用性；不是发布物，但打包时一起给出）
    let pg = root.join(&rel).join("liquid-verre-playground.exe");
    if pg.exists() {
        out.push((PathBuf::from("bin/liquid-verre-playground.exe"), pg));
    }
    // 元数据
    let readme = root.join("README.md");
    if readme.exists() {
        out.push((PathBuf::from("README.md"), readme));
    }
    let license = root.join("LICENSE");
    if license.exists() {
        out.push((PathBuf::from("LICENSE"), license));
    }
    // 着色器源（库运行期用 include_str! 内嵌，这里附带只为便于上层排查）
    let shader_dir = root.join("crates/liquid-verre/shaders");
    if shader_dir.is_dir() {
        for entry in std::fs::read_dir(&shader_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.extension().map(|e| e == "hlsl").unwrap_or(false) {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                out.push((PathBuf::from("shaders").join(name), p));
            }
        }
    }
    if out.is_empty() {
        return Err("没有找到任何可打包产物，请先 `cargo xtask build`".into());
    }
    Ok(out)
}

fn git_sha(root: &Path) -> (String, bool) {
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    (sha, dirty)
}

fn crate_version(root: &Path) -> String {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.trim_start().starts_with("version"))
                .and_then(|l| l.split('"').nth(1))
                .map(|v| v.to_string())
        })
        .unwrap_or_else(|| "0.0.0".into())
}

fn cmd_package(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let mut out_dir = root.join("dist");
    let mut zip = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => {
                if let Some(v) = it.next() {
                    out_dir = root.join(v);
                }
            }
            "--no-zip" => zip = false,
            other => return Err(format!("package: 未知参数 {other}")),
        }
    }

    cmd_build(&[])?;

    let version = crate_version(&root);
    let triple = target_triple();
    let stage = out_dir.join(format!("liquid-verre-rs-{version}-{triple}"));
    if stage.exists() {
        std::fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    let (sha, dirty) = git_sha(&root);
    let artifacts = collect_artifacts(&root)?;
    let mut manifest_lines = Vec::new();
    for (dst, src) in &artifacts {
        let target = stage.join(dst);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(src, &target).map_err(|e| format!("复制 {} 失败: {e}", src.display()))?;
        let size = std::fs::metadata(target).map(|m| m.len()).unwrap_or(0);
        manifest_lines.push(format!(
            "  {{\"path\": \"{}\", \"bytes\": {size}}}",
            dst.to_string_lossy().replace('\\', "/")
        ));
    }

    let info = format!(
        "{{\n  \"name\": \"liquid-verre-rs\",\n  \"version\": \"{version}\",\n  \"target\": \"{triple}\",\n  \"gitSha\": \"{sha}\",\n  \"gitDirty\": {dirty},\n  \"builtAtUtc\": \"{now}\",\n  \"profile\": \"release\",\n  \"artifacts\": [\n{files}\n  ]\n}}\n",
        now = now_utc(),
        files = manifest_lines.join(",\n"),
    );
    std::fs::write(stage.join("build-info.json"), info).map_err(|e| e.to_string())?;

    println!("\n[xtask] 打包目录: {}", stage.display());
    if !zip {
        return Ok(());
    }
    // 压缩交给 Node 脚本（零 npm 依赖、跨平台）。
    // 不用 PowerShell 的 Compress-Archive：只在 Windows 上有，而且引号/编码坑多；
    // 也不引 zip crate：xtask 的价值就在于依赖图为空。
    let archive = out_dir.join(format!("liquid-verre-rs-{version}-{triple}.zip"));
    if archive.exists() {
        std::fs::remove_file(&archive).map_err(|e| e.to_string())?;
    }
    let zip_script = root.join("scripts").join("zip.mjs");
    if !zip_script.exists() {
        println!(
            "[xtask] 找不到 {}，已保留未压缩目录: {}",
            zip_script.display(),
            stage.display()
        );
        return Ok(());
    }
    let mut node = Command::new(node_exe());
    node.arg(&zip_script)
        .arg(&stage)
        .arg(&archive)
        .current_dir(&root);
    run(&mut node)?;
    println!("[xtask] 压缩包: {}", archive.display());
    Ok(())
}

/// Node 可执行文件名（可用 `NODE` 环境变量覆盖）。
fn node_exe() -> PathBuf {
    PathBuf::from(std::env::var("NODE").unwrap_or_else(|_| "node".into()))
}

/// 目标三元组：读 `rustc -vV` 的 `host:` 行。
///
/// 不要硬编码兜底值 —— 同一台 Windows 上可能是 `x86_64-pc-windows-msvc`
/// 也可能是 `x86_64-pc-windows-gnu`，写死会让 `build-info.json` 与真实产物对不上。
fn target_triple() -> String {
    std::env::var("TARGET_TRIPLE")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            Command::new("rustc")
                .arg("-vV")
                .output()
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).into_owned().into())
                .and_then(|text| {
                    text.lines()
                        .find(|l| l.starts_with("host:"))
                        .map(|l| l.trim_start_matches("host:").trim().to_string())
                })
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| "unknown-target".to_string())
}

fn now_utc() -> String {
    // 不引 chrono、也不 shell 到 PowerShell：纯 std 从 UNIX 秒数算 civil 日期
    // （Howard Hinnant 的 civil_from_days 算法，1970 前后都正确）。
    let Ok(now) = std::time::SystemTime::UNIX_EPOCH.elapsed() else {
        return "unknown".into();
    };
    let secs = now.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

// ---------------------------------------------------------------- gate

fn cmd_gate(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    cmd_build(&[])?;
    let script = root.join("tools").join("gate_perf.py");
    if !script.exists() {
        return Err(format!("找不到性能阀门脚本: {}", script.display()));
    }
    let mut c = Command::new(python());
    c.arg(&script).args(args).current_dir(&root);
    run(&mut c)
}

// ---------------------------------------------------------------- clean

fn cmd_clean() -> Result<(), String> {
    let root = workspace_root();
    let mut c = cargo();
    c.arg("clean").current_dir(&root);
    run(&mut c)?;
    let dist = root.join("dist");
    if dist.exists() {
        std::fs::remove_dir_all(&dist).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn usage() {
    eprintln!(
        "cargo xtask <cmd>\n\n  build [--debug]     构建 workspace（默认 release）\n  check               fmt --check + clippy -D warnings + test --release\n  package [--out DIR] [--no-zip]\n                      构建并把产物收集到 dist/，附 build-info.json\n  gate [-- <args>]    跑 tools/gate_perf.py 性能硬门槛\n  clean               cargo clean + 删 dist/"
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => {
            usage();
            return ExitCode::from(2);
        }
    };
    let result = match cmd {
        "build" => cmd_build(rest),
        "check" => cmd_check(),
        "package" => cmd_package(rest),
        "gate" => {
            // `cargo xtask gate -- 12` 里 cargo 会吃掉 `--`，这里只透传剩余参数。
            let rest = rest
                .iter()
                .skip_while(|a| a.as_str() == "--")
                .cloned()
                .collect::<Vec<_>>();
            cmd_gate(&rest)
        }
        "clean" => cmd_clean(),
        "help" | "--help" | "-h" => {
            usage();
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("[xtask] 未知命令: {other}");
            usage();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[xtask] {e}");
            ExitCode::FAILURE
        }
    }
}
