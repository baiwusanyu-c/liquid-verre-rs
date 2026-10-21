//! 日志门面。
//!
//! 库内部一律用 `log` 宏；**不**自带日志后端，由上层应用决定落地方式
//! （`env_logger` / `tracing-log` / 自己的 UI 日志面板都可以）。
//!
//! 为了不强迫上层先接一个 logger 才能看到诊断输出，这里提供一个可选的默认
//! stderr logger（[`install_default_logger`]）。它的**行格式与重构前逐字节一致**：
//!
//! ```text
//! [boot] 创建 D3D11 设备...
//! [info] 显示器: ...
//! [warn] ... 采集不可用，跳过该屏: ...
//! [verify] ...
//! [metrics] win_s=... fps=... stage_us=A.. B.. P.. C.. ...
//! ```
//!
//! `tools/perf_probe.py`、`tools/gate_perf.py`、`tools/verify_*.py` 全部依赖这些前缀
//! 与字段名解析，**改格式等于打断质量阀门**（见
//! `.trellis/spec/backend/logging-guidelines.md`）。
//!
//! 前缀由 `log` 的 `target` 决定：`target: "boot"` → `[boot]`，
//! 默认 target（模块路径）→ `[info]` / `[warn]` / `[error]` / `[debug]` / `[trace]`。

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 特殊 target：这些 target 的名字**就是**要打印的前缀，不再叠加级别名。
const RAW_TARGETS: &[&str] = &["boot", "verify", "metrics", "adaptive"];

struct StderrLogger {
    level: log::LevelFilter,
}

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let target = record.target();
        let prefix: &str = match RAW_TARGETS.iter().position(|t| *t == target) {
            Some(_) => target,
            None => match record.level() {
                log::Level::Error => "error",
                log::Level::Warn => "warn",
                log::Level::Info => "info",
                log::Level::Debug => "debug",
                log::Level::Trace => "trace",
            },
        };
        // 单行输出：stderr 上一次 write_all，避免多线程交错撕裂 metrics 行。
        let line = format!("[{prefix}] {}\n", record.args());
        let stderr = std::io::stderr();
        let mut lock = stderr.lock();
        let _ = lock.write_all(line.as_bytes());
        let _ = lock.flush();
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

static DEFAULT_LOGGER: StderrLogger = StderrLogger {
    level: log::LevelFilter::Info,
};
static INSTALLED: AtomicUsize = AtomicUsize::new(0);

/// 安装默认 stderr logger（幂等；上层已安装自己的 logger 时不覆盖）。
///
/// 返回 `true` 表示本次调用真的装上了；`false` 表示已有 logger（可能是上层的）。
pub fn install_default_logger() -> bool {
    install_default_logger_with_level(log::LevelFilter::Info)
}

/// 同 [`install_default_logger`]，但可指定最低级别。
pub fn install_default_logger_with_level(level: log::LevelFilter) -> bool {
    if INSTALLED.swap(1, Ordering::AcqRel) != 0 {
        return false;
    }
    // 级别存在静态里没法改，因此这里用 set_max_level 控制全局过滤，
    // logger 自身按 record.level() 判断即可。
    log::set_max_level(level);
    log::set_logger(&DEFAULT_LOGGER).is_ok()
}

/// 库内部：把 `log` 的最大级别抬到指定值（上层没装 logger 时也能看到 debug）。
pub(crate) fn set_max_level(level: log::LevelFilter) {
    log::set_max_level(level);
}

#[cfg(test)]
mod tests {
    use super::RAW_TARGETS;

    #[test]
    fn raw_targets_cover_metrics_prefix() {
        // tools/perf_probe.py 靠 "[metrics]" 认行，这个 target 必须在白名单里
        assert!(RAW_TARGETS.contains(&"metrics"));
        assert!(RAW_TARGETS.contains(&"boot"));
        assert!(RAW_TARGETS.contains(&"verify"));
        // tools/verify_adaptive.py 靠 "[adaptive]" 认行
        assert!(RAW_TARGETS.contains(&"adaptive"));
    }
}
