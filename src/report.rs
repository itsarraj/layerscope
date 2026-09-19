use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::{manifest, tarutil};

#[derive(Debug, Clone, PartialEq)]
pub struct LayerReport {
    pub path: String,
    /// Sum of real (non-whiteout) file sizes in this layer, uncompressed.
    /// Deliberately not the compressed blob size on disk — see README for
    /// why that's the more useful number for "what's actually bloating
    /// this layer," and how it differs from `docker history`'s number.
    pub content_size: u64,
    pub file_count: usize,
    pub whiteout_count: usize,
    /// Largest files in this layer, descending, truncated to the
    /// caller-requested `top_n`.
    pub largest_files: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageReport {
    pub repo_tags: Vec<String>,
    pub layers: Vec<LayerReport>,
    pub total_content_size: u64,
}

pub fn analyze_layer(path: &str, raw_bytes: &[u8], top_n: usize) -> Result<LayerReport> {
    let decompressed = tarutil::maybe_decompress(raw_bytes)?;
    let sizes = tarutil::scan_tar_sizes(&decompressed)?;
    let content_size: u64 = sizes.files.iter().map(|(_, s)| s).sum();

    let mut largest = sizes.files.clone();
    largest.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    largest.truncate(top_n);

    Ok(LayerReport {
        path: path.to_string(),
        content_size,
        file_count: sizes.files.len(),
        whiteout_count: sizes.whiteouts.len(),
        largest_files: largest,
    })
}

pub fn analyze_image_tar(
    entries: &HashMap<String, Vec<u8>>,
    top_n: usize,
) -> Result<Vec<ImageReport>> {
    let manifest_bytes = entries.get("manifest.json").context(
        "no manifest.json in this tar — only the classic `docker save` format is supported (see README)",
    )?;
    let manifests = manifest::parse_manifest(manifest_bytes)?;

    let mut reports = Vec::new();
    for m in manifests {
        let mut layers = Vec::new();
        for layer_path in &m.layers {
            let raw = entries.get(layer_path).with_context(|| {
                format!("layer '{layer_path}' is listed in manifest.json but missing from the tar")
            })?;
            layers.push(analyze_layer(layer_path, raw, top_n)?);
        }
        let total_content_size = layers.iter().map(|l| l.content_size).sum();
        reports.push(ImageReport {
            repo_tags: m.repo_tags,
            layers,
            total_content_size,
        });
    }
    Ok(reports)
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit_idx])
    }
}

pub fn render_text(report: &ImageReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let tags = if report.repo_tags.is_empty() {
        "<untagged>".to_string()
    } else {
        report.repo_tags.join(", ")
    };
    writeln!(out, "image: {tags}").ok();
    writeln!(
        out,
        "total content size: {} across {} layers\n",
        format_bytes(report.total_content_size),
        report.layers.len()
    )
    .ok();

    for (i, layer) in report.layers.iter().enumerate() {
        writeln!(
            out,
            "layer {i} ({}): {} in {} files{}",
            layer.path,
            format_bytes(layer.content_size),
            layer.file_count,
            if layer.whiteout_count > 0 {
                format!(", {} whiteout(s)", layer.whiteout_count)
            } else {
                String::new()
            }
        )
        .ok();
        for (path, size) in &layer.largest_files {
            writeln!(out, "    {:>10}  {path}", format_bytes(*size)).ok();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_picks_a_sensible_unit() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
    }

    #[test]
    fn analyze_layer_sums_real_files_and_excludes_whiteouts_from_size() {
        let mut builder = tar::Builder::new(Vec::new());
        let add = |b: &mut tar::Builder<Vec<u8>>, name: &str, contents: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            b.append_data(&mut header, name, contents).unwrap();
        };
        add(&mut builder, "app/main.bin", &vec![0u8; 1000]);
        add(&mut builder, "app/.wh.old_binary", b"");
        let layer_bytes = builder.into_inner().unwrap();

        let report = analyze_layer("layer1/layer.tar", &layer_bytes, 5).unwrap();
        assert_eq!(
            report.content_size, 1000,
            "whiteout marker must not count toward content size"
        );
        assert_eq!(report.file_count, 1);
        assert_eq!(report.whiteout_count, 1);
        assert_eq!(report.largest_files[0].0, "app/main.bin");
    }

    #[test]
    fn largest_files_truncated_and_sorted_descending() {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, size) in [("small", 10), ("big", 1000), ("medium", 100)] {
            let mut header = tar::Header::new_gnu();
            header.set_size(size);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, vec![0u8; size as usize].as_slice())
                .unwrap();
        }
        let layer_bytes = builder.into_inner().unwrap();

        let report = analyze_layer("l/layer.tar", &layer_bytes, 2).unwrap();
        assert_eq!(report.largest_files.len(), 2, "must truncate to top_n");
        assert_eq!(report.largest_files[0].0, "big");
        assert_eq!(report.largest_files[1].0, "medium");
    }
}
