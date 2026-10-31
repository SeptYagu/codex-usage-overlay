//! Update channel selection.
//!
//! The stable channel uses the endpoint in `tauri.conf.json`, GitHub's
//! `releases/latest`, which never names a prerelease. With prereleases
//! enabled, the newest published release (stable or preview) is looked up
//! through the GitHub API and its `latest.json` is checked instead. The
//! manifest is still verified by the updater's signature check, so the API
//! only chooses which signed manifest to read.
use serde::Deserialize;
use std::time::Duration;

const RELEASES_API: &str = "https://api.github.com/repos/SeptYagu/codex-usage-overlay/releases?per_page=30";
const DOWNLOAD_PREFIX: &str = "https://github.com/SeptYagu/codex-usage-overlay/releases/download/";
const MANIFEST: &str = "latest.json";
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// The manifest of the highest-versioned published release that carries one.
/// A release still uploading its assets has no manifest yet and is skipped.
pub fn newest_manifest_url(releases: &[Release]) -> Option<String> {
    releases
        .iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = semver::Version::parse(release.tag_name.trim_start_matches('v')).ok()?;
            let manifest = release.assets.iter().find(|asset| asset.name == MANIFEST)?;
            // Only manifests of this repository are ever handed to the updater.
            manifest
                .browser_download_url
                .starts_with(DOWNLOAD_PREFIX)
                .then(|| (version, manifest.browser_download_url.clone()))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, url)| url)
}

pub async fn prerelease_endpoint() -> Result<tauri::Url, String> {
    // The updater installs the same provider before its own requests.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("codex-usage-overlay/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let releases: Vec<Release> = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("Could not list releases: {error}"))?
        .json()
        .await
        .map_err(|error| format!("Could not read the release list: {error}"))?;
    let url = newest_manifest_url(&releases).ok_or("No published release has an update manifest")?;
    url.parse().map_err(|error| format!("Invalid update manifest URL: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, draft: bool, manifest_url: Option<&str>) -> Release {
        Release {
            tag_name: tag.into(),
            draft,
            assets: manifest_url
                .map(|url| vec![
                    Asset { name: "SHA256SUMS.txt".into(), browser_download_url: format!("{DOWNLOAD_PREFIX}{tag}/SHA256SUMS.txt") },
                    Asset { name: MANIFEST.into(), browser_download_url: url.into() },
                ])
                .unwrap_or_default(),
        }
    }

    #[test]
    fn the_highest_published_version_with_a_manifest_wins() {
        let url = |tag: &str| format!("{DOWNLOAD_PREFIX}{tag}/latest.json");
        let releases = vec![
            // Newer by creation date, but a lower version than the preview.
            release("v1.4.2", false, Some(&url("v1.4.2"))),
            release("v1.5.0-preview", false, Some(&url("v1.5.0-preview"))),
            release("v1.4.1", false, Some(&url("v1.4.1"))),
            release("v1.6.0-preview", true, Some(&url("v1.6.0-preview"))),
            release("v1.7.0-preview", false, None),
            release("nightly", false, Some(&url("nightly"))),
        ];
        assert_eq!(newest_manifest_url(&releases), Some(url("v1.5.0-preview")));
        // A stable release supersedes its own preview.
        let mut with_stable = releases;
        with_stable.push(release("v1.5.0", false, Some(&url("v1.5.0"))));
        assert_eq!(newest_manifest_url(&with_stable), Some(url("v1.5.0")));
    }

    #[test]
    fn manifests_outside_this_repository_are_ignored() {
        let releases = vec![release("v9.0.0", false, Some("https://example.com/latest.json"))];
        assert_eq!(newest_manifest_url(&releases), None);
        assert_eq!(newest_manifest_url(&[]), None);
    }

    #[test]
    fn the_github_release_list_shape_deserializes() {
        let raw = r#"[{"tag_name":"v1.5.0-preview","draft":false,"prerelease":true,
            "assets":[{"name":"latest.json","browser_download_url":"https://github.com/SeptYagu/codex-usage-overlay/releases/download/v1.5.0-preview/latest.json","size":1}]}]"#;
        let releases: Vec<Release> = serde_json::from_str(raw).unwrap();
        assert!(newest_manifest_url(&releases).unwrap().ends_with("/v1.5.0-preview/latest.json"));
    }
}
