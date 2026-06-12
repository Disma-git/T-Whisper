//! Uppdateringskoll mot GitHub Releases. Frågar bara efter senaste
//! versionsnumret — laddar aldrig ner något själv.

use std::time::Duration;

const LATEST_API: &str = "https://api.github.com/repos/Disma-git/T-Whisper/releases/latest";

/// Hämtar senaste release-taggen (t.ex. "v0.4.0") från GitHub.
/// Fel returneras som Err och ska bara loggas — aldrig störa användaren.
pub fn fetch_latest_tag() -> anyhow::Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        // GitHub-API:t kräver en User-Agent.
        .user_agent(format!("T-Whisper/{}", env!("CARGO_PKG_VERSION")))
        .build()?;
    let body: serde_json::Value = client.get(LATEST_API).send()?.error_for_status()?.json()?;
    body["tag_name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("tag_name saknas i svaret"))
}

/// True om `latest` (t.ex. "v0.4.1") är nyare än `current` (t.ex. "0.4.0").
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    }
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv != cv {
            return lv > cv;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versionsjamforelse() {
        assert!(is_newer("v0.4.1", "0.4.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("v0.4.0.1", "0.4.0"));
        assert!(!is_newer("v0.4.0", "0.4.0"));
        assert!(!is_newer("v0.3.9", "0.4.0"));
        assert!(!is_newer("skräp", "0.4.0"));
    }
}
