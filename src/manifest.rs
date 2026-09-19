use anyhow::{Context, Result};
use serde::Deserialize;

/// Matches the classic `docker save` `manifest.json` shape: a JSON array,
/// one object per image in the tarball. OCI-layout images (`index.json` +
/// per-blob `manifest.json`) are a different, not-yet-supported shape —
/// see README.
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestEntry {
    #[serde(rename = "Config")]
    pub config: String,
    #[serde(rename = "RepoTags", default)]
    pub repo_tags: Vec<String>,
    #[serde(rename = "Layers")]
    pub layers: Vec<String>,
}

pub fn parse_manifest(bytes: &[u8]) -> Result<Vec<ManifestEntry>> {
    serde_json::from_slice(bytes)
        .context("parsing manifest.json (expected the classic `docker save` array shape)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_typical_docker_save_manifest() {
        let json = br#"[
            {
                "Config": "abc123.json",
                "RepoTags": ["myapp:latest"],
                "Layers": ["layer1/layer.tar", "layer2/layer.tar"]
            }
        ]"#;
        let manifests = parse_manifest(json).unwrap();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].repo_tags, vec!["myapp:latest"]);
        assert_eq!(
            manifests[0].layers,
            vec!["layer1/layer.tar", "layer2/layer.tar"]
        );
    }

    #[test]
    fn missing_repo_tags_defaults_to_empty_rather_than_failing() {
        // A dangling/untagged image's manifest entry has no "RepoTags" key
        // at all, not an empty array — `#[serde(default)]` is what keeps
        // that from being a parse error.
        let json = br#"[{"Config": "abc.json", "Layers": ["l/layer.tar"]}]"#;
        let manifests = parse_manifest(json).unwrap();
        assert!(manifests[0].repo_tags.is_empty());
    }
}
