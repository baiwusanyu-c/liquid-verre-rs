//! 构建期元数据（版本号 / git sha / 目标三元组 / 内嵌着色器摘要）。
//!
//! 常量由 `build.rs` 生成到 `OUT_DIR/build_info.rs`，此处 `include!` 后包一层
//! 稳定结构体对外暴露。用途：上层应用在崩溃上报 / 关于面板里带上「用的是哪一版库、
//! 哪一份着色器」，排查驱动相关问题时不必猜。

include!(concat!(env!("OUT_DIR"), "/build_info.rs"));

/// 构建期元数据快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// 库版本（`CARGO_PKG_VERSION`）。
    pub version: &'static str,
    /// 构建时 git 短 sha；无 git 环境为 `"unknown"`。
    pub git_sha: &'static str,
    /// 构建时工作区是否有未提交改动。
    pub git_dirty: bool,
    /// 构建时间（UTC，ISO8601）。
    pub built_at_utc: &'static str,
    /// 目标三元组。
    pub target_triple: &'static str,
    /// rustc 版本。
    pub rustc_version: &'static str,
    /// 构建 profile（`debug` / `release`）。
    pub profile: &'static str,
    /// 内嵌着色器的 `(文件名, sha256 前 12 位)`，按文件名升序。
    pub shaders: &'static [(&'static str, &'static str)],
}

/// 本二进制的构建元数据。
pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: VERSION,
    git_sha: GIT_SHA,
    git_dirty: GIT_DIRTY,
    built_at_utc: BUILD_TIMESTAMP_UTC,
    target_triple: TARGET_TRIPLE,
    rustc_version: RUSTC_VERSION,
    profile: PROFILE,
    shaders: SHADERS,
};

impl BuildInfo {
    /// 单行摘要，适合直接打进日志或崩溃上报。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "liquid-verre-rs {} ({}{} {} {} {})",
            self.version,
            self.git_sha,
            if self.git_dirty { "-dirty" } else { "" },
            self.profile,
            self.target_triple,
            self.rustc_version,
        )
    }
}

impl std::fmt::Display for BuildInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.summary())
    }
}

#[cfg(test)]
mod tests {
    use super::BUILD_INFO;

    #[test]
    fn version_is_not_placeholder() {
        assert!(!BUILD_INFO.version.is_empty());
        assert!(
            BUILD_INFO.version.contains('.'),
            "version={}",
            BUILD_INFO.version
        );
    }

    #[test]
    fn shaders_are_embedded_with_digests() {
        // 6 个 hlsl：glass_vs / glass_ps / blur_vs / blur_down / blur_up / env_ema
        assert!(
            BUILD_INFO.shaders.len() >= 6,
            "got {:?}",
            BUILD_INFO.shaders
        );
        let mut sorted = BUILD_INFO
            .shaders
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>();
        let mut expected = sorted.clone();
        expected.sort_unstable();
        assert_eq!(sorted, expected, "着色器清单必须按文件名升序");
        sorted.sort_unstable();
        for (_, digest) in BUILD_INFO.shaders {
            assert_eq!(digest.len(), 12, "sha256 前缀应为 12 位: {digest}");
            assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn summary_mentions_crate_name() {
        assert!(BUILD_INFO.summary().contains("liquid-verre-rs"));
    }
}
