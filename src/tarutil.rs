use std::collections::HashMap;
use std::io::{Cursor, Read};

use anyhow::{Context, Result};

/// Gzip magic bytes — `docker save`'s outer tar and its classic
/// `layer.tar` entries are uncompressed, but OCI-layout blobs
/// (`application/...tar+gzip`) are gzip-compressed. Checking the magic
/// bytes rather than trusting a file extension means both work through the
/// same code path with no format flag needed from the caller.
fn is_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

pub fn maybe_decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_gzip(bytes) {
        let mut decoder = flate2::read::GzDecoder::new(bytes);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .context("gzip-decompressing tar entry")?;
        Ok(out)
    } else {
        Ok(bytes.to_vec())
    }
}

/// Reads every regular-file entry of a tar archive into memory, keyed by
/// its path within the archive. This is what makes the rest of this crate
/// pure/testable: everything downstream operates on a `HashMap<String,
/// Vec<u8>>` that a test can build in-memory with `tar::Builder`, with the
/// real `docker save` tarball and this in-memory fixture going through the
/// identical code path.
pub fn read_tar_entries(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let mut result = HashMap::new();
    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("reading a tar entry")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .context("reading entry path")?
            .to_string_lossy()
            .into_owned();
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .context("reading entry contents")?;
        result.insert(path, contents);
    }
    Ok(result)
}

/// Per-file sizes within a tar, without holding file contents in memory —
/// used for layer tars, which can be large and where only sizes matter.
/// Also reports which entries are AUFS-style whiteout markers
/// (`.wh.<name>` for a deleted file, `.wh..wh..opq` for an opaque
/// directory) since those aren't "content", they're delete-markers.
pub struct TarSizes {
    pub files: Vec<(String, u64)>,
    pub whiteouts: Vec<String>,
}

const WHITEOUT_PREFIX: &str = ".wh.";
const OPAQUE_MARKER: &str = ".wh..wh..opq";

pub fn scan_tar_sizes(bytes: &[u8]) -> Result<TarSizes> {
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let mut files = Vec::new();
    let mut whiteouts = Vec::new();

    for entry in archive.entries().context("reading tar entries")? {
        let entry = entry.context("reading a tar entry")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .context("reading entry path")?
            .to_string_lossy()
            .into_owned();
        let size = entry.header().size().unwrap_or(0);

        let file_name = path.rsplit('/').next().unwrap_or(&path);
        if file_name == OPAQUE_MARKER || file_name.starts_with(WHITEOUT_PREFIX) {
            whiteouts.push(path);
        } else {
            files.push((path, size));
        }
    }
    Ok(TarSizes { files, whiteouts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn build_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *contents).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn read_tar_entries_collects_paths_and_contents() {
        let tar_bytes = build_tar(&[("a.txt", b"hello"), ("dir/b.txt", b"world!")]);
        let entries = read_tar_entries(&tar_bytes).unwrap();
        assert_eq!(entries.get("a.txt").unwrap(), b"hello");
        assert_eq!(entries.get("dir/b.txt").unwrap(), b"world!");
    }

    #[test]
    fn scan_tar_sizes_separates_whiteouts_from_real_files() {
        let tar_bytes = build_tar(&[
            ("real_file.txt", b"some content here"),
            ("some/dir/.wh.deleted_file", b""),
            ("some/dir/.wh..wh..opq", b""),
        ]);
        let sizes = scan_tar_sizes(&tar_bytes).unwrap();
        assert_eq!(sizes.files.len(), 1);
        assert_eq!(sizes.files[0].0, "real_file.txt");
        assert_eq!(sizes.files[0].1, "some content here".len() as u64);
        assert_eq!(sizes.whiteouts.len(), 2);
    }

    #[test]
    fn gzip_roundtrip_is_detected_and_decompressed() {
        let plain = build_tar(&[("x", b"y")]);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&plain).unwrap();
        let compressed = encoder.finish().unwrap();

        assert!(is_gzip(&compressed));
        assert!(!is_gzip(&plain));

        let decompressed = maybe_decompress(&compressed).unwrap();
        assert_eq!(decompressed, plain);

        // Plain (non-gzip) input must pass through unchanged.
        assert_eq!(maybe_decompress(&plain).unwrap(), plain);
    }
}
