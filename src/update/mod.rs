//! Update check functionality

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::session::get_app_dir;

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub body: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCache {
    checked_at: chrono::DateTime<chrono::Utc>,
    latest_version: String,
    #[serde(default)]
    releases: Vec<ReleaseInfo>,
}

fn cache_path() -> Result<PathBuf> {
    Ok(get_app_dir()?.join("update_cache.json"))
}

fn load_cache() -> Option<UpdateCache> {
    let path = cache_path().ok()?;
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

pub async fn check_for_update(current_version: &str, _force: bool) -> Result<UpdateInfo> {
    // Upstream update checks are disabled for this fork.
    // Always report no update available.
    Ok(UpdateInfo {
        available: false,
        current_version: current_version.to_string(),
        latest_version: current_version.to_string(),
    })
}

/// Get cached release notes, filtered to show only releases newer than from_version.
/// Returns releases in newest-first order.
pub fn get_cached_releases(from_version: Option<&str>) -> Vec<ReleaseInfo> {
    let cache = match load_cache() {
        Some(c) => c,
        None => return vec![],
    };

    filter_releases(cache.releases, from_version)
}

fn filter_releases(releases: Vec<ReleaseInfo>, from_version: Option<&str>) -> Vec<ReleaseInfo> {
    match from_version {
        Some(from) => releases
            .into_iter()
            .take_while(|r| r.version != from)
            .collect(),
        None => releases,
    }
}

#[cfg(test)]
fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse_version =
        |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse().ok()).collect() };

    let latest_parts = parse_version(latest);
    let current_parts = parse_version(current);

    for i in 0..latest_parts.len().max(current_parts.len()) {
        let l = latest_parts.get(i).copied().unwrap_or(0);
        let c = current_parts.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }
    false
}

pub async fn print_update_notice() {
    // Upstream update checks are disabled for this fork.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert!(is_newer_version("1.0.1", "1.0.0"));
        assert!(is_newer_version("1.1.0", "1.0.9"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
    }

    #[test]
    fn test_cache_should_invalidate_when_current_newer_than_cached() {
        let cached_latest = "0.4.5";
        let current_version = "0.5.0";

        let current_is_newer = is_newer_version(current_version, cached_latest);
        assert!(current_is_newer, "0.5.0 should be newer than 0.4.5");

        let same_version = is_newer_version("0.4.5", "0.4.5");
        assert!(
            !same_version,
            "same version should not trigger invalidation"
        );

        let downgrade = is_newer_version("0.4.0", "0.4.5");
        assert!(!downgrade, "downgrade should not trigger invalidation");
    }

    fn make_release(version: &str) -> ReleaseInfo {
        ReleaseInfo {
            version: version.to_string(),
            body: format!("Release notes for {}", version),
            published_at: None,
        }
    }

    #[test]
    fn test_filter_releases_returns_all_when_no_filter() {
        let releases = vec![
            make_release("0.5.0"),
            make_release("0.4.3"),
            make_release("0.4.2"),
        ];

        let filtered = filter_releases(releases.clone(), None);

        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0].version, "0.5.0");
        assert_eq!(filtered[1].version, "0.4.3");
        assert_eq!(filtered[2].version, "0.4.2");
    }

    #[test]
    fn test_filter_releases_stops_at_from_version() {
        let releases = vec![
            make_release("0.5.0"),
            make_release("0.4.3"),
            make_release("0.4.2"),
            make_release("0.4.1"),
        ];

        let filtered = filter_releases(releases, Some("0.4.3"));

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].version, "0.5.0");
    }

    #[test]
    fn test_filter_releases_returns_empty_when_from_version_is_latest() {
        let releases = vec![make_release("0.5.0"), make_release("0.4.3")];

        let filtered = filter_releases(releases, Some("0.5.0"));

        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_releases_returns_all_when_from_version_not_found() {
        let releases = vec![make_release("0.5.0"), make_release("0.4.3")];

        let filtered = filter_releases(releases.clone(), Some("0.3.0"));

        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_releases_handles_empty_list() {
        let releases: Vec<ReleaseInfo> = vec![];

        let filtered = filter_releases(releases.clone(), Some("0.4.3"));
        assert!(filtered.is_empty());

        let filtered = filter_releases(releases, None);
        assert!(filtered.is_empty());
    }
}
