//! 统一错误类型。
//!
//! 设计目标（库项目）：
//! - 上层应用可以 **`match` 到具体类别**（`Error::kind()`）并据此决定重试 / 降级 / 上报；
//! - 保留底层 HRESULT / Win32 文本作为 `source()`，排查时不丢信息；
//! - 构造只在失败分支发生，**热路径零额外分配**；
//! - 内部大量既有代码用 `Result<_, String>`，因此提供 `From<String>` / `From<&str>`，
//!   让 `?` 平滑收敛到本类型（类别退化为 `Internal`，需要精确类别的点位显式构造）。

use std::fmt;

/// 错误类别。上层按这个枚举做分支，不要匹配字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// D3D11 设备 / DXGI 工厂创建失败（含回退 WARP 后仍失败）。
    GraphicsInit,
    /// HLSL 编译或 D3D11 着色器 / 状态对象创建失败。
    ShaderCompile,
    /// 显示器枚举失败（`EnumDisplayMonitors` 返回空或调用失败）。
    MonitorEnumeration,
    /// 画布窗口 / 窗口类创建失败。
    WindowCreation,
    /// 桌面采集不可用（DDA 与 WGC 都拿不到）。
    ///
    /// 注意：**单屏**采集失败不致命（该屏跳过画布并按退避重试，走
    /// `EngineEvent::Warning`）；只有「一块屏都建不出画布」才返回 `Err`，
    /// 而那条路径的类别是 `WindowCreation`（独立审查 OVERDESIGN #3）。
    CaptureUnavailable,
    /// 采集运行期失效（锁屏 / 独占全屏 / 被其它进程抢占）。
    /// **不会**作为 `Err` 返回：库内部按 500 ms 退避自动重建，
    /// 只通过 `EngineEvent::Warning` 上抛，避免上层被刷屏。
    CaptureLost,
    /// 上层传入的配置非法（已被 `sanitize` 夹紧但仍不可用，例如尺寸为 0）。
    InvalidConfig,
    /// 引擎已关闭后再调用句柄方法。
    Shutdown,
    /// 其它未分类错误。
    Internal,
}

impl ErrorKind {
    /// 稳定的英文标识符，用于日志 / 序列化（不随本地化变化）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::GraphicsInit => "graphics-init",
            ErrorKind::ShaderCompile => "shader-compile",
            ErrorKind::MonitorEnumeration => "monitor-enumeration",
            ErrorKind::WindowCreation => "window-creation",
            ErrorKind::CaptureUnavailable => "capture-unavailable",
            ErrorKind::CaptureLost => "capture-lost",
            ErrorKind::InvalidConfig => "invalid-config",
            ErrorKind::Shutdown => "shutdown",
            ErrorKind::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 库的统一错误类型。
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

/// 库的统一 `Result` 别名。
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// 用类别 + 消息构造。
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// 附带底层错误源（保留 HRESULT 文本）。
    pub fn with_source(
        kind: ErrorKind,
        message: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            source: Some(source.into()),
        }
    }

    /// 错误类别。
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// 人类可读消息（不含类别前缀）。
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 把类别改写成更精确的值（用于把 `Internal` 细化）。
    #[must_use]
    pub fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.kind, self.message)?;
        if let Some(src) = &self.source {
            write!(f, ": {src}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|b| &**b as &(dyn std::error::Error + 'static))
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        let code = e.code();
        Self::with_source(
            ErrorKind::Internal,
            format!("Win32/COM 调用失败 ({code:?})"),
            e,
        )
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::with_source(ErrorKind::Internal, "I/O 失败", e)
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, ErrorKind};

    #[test]
    fn display_includes_kind_and_message() {
        let e = Error::new(ErrorKind::ShaderCompile, "ps_main 编译失败");
        assert_eq!(e.to_string(), "[shader-compile] ps_main 编译失败");
        assert_eq!(e.kind(), ErrorKind::ShaderCompile);
    }

    #[test]
    fn kind_str_is_stable() {
        assert_eq!(ErrorKind::CaptureLost.as_str(), "capture-lost");
        assert_eq!(ErrorKind::GraphicsInit.as_str(), "graphics-init");
    }

    #[test]
    fn from_str_and_string_degrade_to_internal() {
        let a: Error = "boom".into();
        let b: Error = String::from("boom2").into();
        assert_eq!(a.kind(), ErrorKind::Internal);
        assert_eq!(b.message(), "boom2");
    }

    #[test]
    fn source_is_preserved() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let e = Error::with_source(ErrorKind::Internal, "读文件失败", io);
        assert!(std::error::Error::source(&e).is_some());
        assert!(e.to_string().contains("读文件失败"));
    }

    #[test]
    fn question_mark_converts_string_errors() {
        fn inner() -> Result<(), String> {
            Err("inner".to_string())
        }
        fn outer() -> super::Result<()> {
            inner()?;
            Ok(())
        }
        assert_eq!(outer().unwrap_err().message(), "inner");
    }
}
