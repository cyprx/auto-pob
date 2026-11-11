//! The opt-in release notice (R22).
//!
//! Off unless `AUTO_POB_UPDATE_CHECK=1`. When on, it makes one GET to the GitHub releases API
//! on a background thread and says "auto-pob vX is available" with a link. It sends only a
//! User-Agent of `auto-pob/<version>`, downloads nothing and never touches PoB or its folder.
//! A failed or offline check is silent.

use anyhow::{Context as _, bail};
use eframe::egui;
use serde::Deserialize;
use std::sync::mpsc;
use std::time::Duration;

pub const RELEASES_URL: &str = "https://api.github.com/repos/cyprx/auto-pob/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    html_url: String,
}

/// "v1.2.3" or "1.2.3" as numbers; anything else is not a version we compare.
fn parse_version(text: &str) -> anyhow::Result<[u64; 3]> {
    let bare = text.strip_prefix(['v', 'V']).unwrap_or(text);
    let mut parts = bare.split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        let part = parts.next().context("fewer than three parts")?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            bail!("{text:?} is not a version");
        }
        *slot = part.parse().context("version number too large")?;
    }
    if parts.next().is_some() {
        bail!("{text:?} is not a version");
    }
    Ok(out)
}

/// The release in `json` (GitHub's "latest release" answer) when it is newer than `current`.
pub fn newer_release(current: &str, json: &str) -> anyhow::Result<Option<Release>> {
    let release: ReleaseJson = serde_json::from_str(json).context("release answer")?;
    if !release.html_url.starts_with("https://") {
        bail!("the release link is not an https link");
    }
    let latest = parse_version(&release.tag_name)?;
    if latest <= parse_version(current)? {
        return Ok(None);
    }
    Ok(Some(Release {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_owned(),
        url: release.html_url,
    }))
}

/// Whether the user opted in (R22).
pub fn enabled() -> bool {
    enabled_for(std::env::var("AUTO_POB_UPDATE_CHECK").ok().as_deref())
}

fn enabled_for(value: Option<&str>) -> bool {
    value == Some("1")
}

fn fetch(current: &str) -> anyhow::Result<Option<Release>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("auto-pob/{current}"))
        .timeout(Duration::from_secs(10))
        .build()?;
    let body = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .text()?;
    newer_release(current, &body)
}

/// Checks once on a thread. The answer arrives only when a newer release exists; every
/// failure is dropped with a debug line.
pub fn spawn_check(ctx: egui::Context) -> mpsc::Receiver<Release> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        match fetch(env!("CARGO_PKG_VERSION")) {
            Ok(Some(release)) => {
                if tx.send(release).is_ok() {
                    ctx.request_repaint();
                }
            }
            Ok(None) => {}
            Err(error) => log::debug!("release check failed: {error:#}"),
        };
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/cyprx/auto-pob/releases/tag/{tag}","name":"x"}}"#
        )
    }

    #[test]
    fn newer_release_compares_versions() {
        let newer = newer_release("0.1.0", &json("v0.2.0")).unwrap().unwrap();
        assert_eq!(newer.version, "0.2.0");
        assert_eq!(
            newer.url,
            "https://github.com/cyprx/auto-pob/releases/tag/v0.2.0"
        );
        assert!(newer_release("0.1.0", &json("0.10.0")).unwrap().is_some());
        assert_eq!(newer_release("0.1.0", &json("v0.1.0")).unwrap(), None);
        assert_eq!(newer_release("0.1.0", &json("v0.0.9")).unwrap(), None);
        assert_eq!(
            newer_release("0.9.0", &json("v0.10.0"))
                .unwrap()
                .map(|r| r.version),
            Some("0.10.0".into())
        );
        assert!(newer_release("0.1.0", &json("nightly")).is_err());
        assert!(newer_release("0.1.0", &json("v1.2")).is_err());
        assert!(newer_release("0.1.0", &json("v1.2.3-beta")).is_err());
    }

    #[test]
    fn release_json_needs_tag_and_url() {
        assert!(newer_release("0.1.0", r#"{"html_url":"https://x/y"}"#).is_err());
        assert!(newer_release("0.1.0", r#"{"tag_name":"v9.0.0"}"#).is_err());
        assert!(newer_release("0.1.0", "not json").is_err());
        // the link is opened in a browser, so only https is accepted
        assert!(
            newer_release("0.1.0", r#"{"tag_name":"v9.0.0","html_url":"file:///etc"}"#).is_err()
        );
    }

    #[test]
    fn check_disabled_by_default() {
        assert!(!enabled_for(None));
        assert!(!enabled_for(Some("")));
        assert!(!enabled_for(Some("0")));
        assert!(!enabled_for(Some("true")));
        assert!(enabled_for(Some("1")));
    }
}
