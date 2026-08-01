//! Version and compatibility information for this port.

/// A machine-readable description of the upstream versions implemented by the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AIogramCompatibility {
    pub port_version: &'static str,
    pub aiogram_version: &'static str,
    pub aiogram_commit: &'static str,
    pub telegram_bot_api_version: &'static str,
    pub telegram_bot_api_release_date: &'static str,
}

/// Compatibility pinned for this build.
pub const COMPATIBILITY: AIogramCompatibility = AIogramCompatibility {
    port_version: env!("CARGO_PKG_VERSION"),
    aiogram_version: "3.30.0",
    aiogram_commit: "c1b0353ce3d3f8d70f90469038939a956e9e09f7",
    telegram_bot_api_version: "10.2",
    telegram_bot_api_release_date: "2026-07-14",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_manifest_matches_compiled_constants() {
        let manifest: toml::Value = toml::from_str(include_str!("../compatibility.toml")).unwrap();
        assert_eq!(
            manifest["port"]["version"].as_str(),
            Some(COMPATIBILITY.port_version)
        );
        assert_eq!(
            manifest["upstream"]["aiogram"]["version"].as_str(),
            Some(COMPATIBILITY.aiogram_version)
        );
        assert_eq!(
            manifest["upstream"]["aiogram"]["commit"].as_str(),
            Some(COMPATIBILITY.aiogram_commit)
        );
        assert_eq!(
            manifest["upstream"]["telegram_bot_api"]["version"].as_str(),
            Some(COMPATIBILITY.telegram_bot_api_version)
        );
    }
}
