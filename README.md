# layerscope

A container image layer/bloat analyzer — Go's `dive` is the standard for
"why is my Docker image so big," and there's no Rust equivalent as far as
I'm aware.

## Usage

```bash
docker save myimage:latest -o image.tar
layerscope image.tar
layerscope --top 20 image.tar   # list more/fewer of the largest files per layer
```

No Docker daemon needed at analysis time — this reads the exported tarball
directly, so it works in this exact sandbox (no `docker.sock` available
here) as well as in CI.

## What it does

For each layer: uncompressed content size, file count, and the N largest
files (path + size) — the actual "what's taking up space" answer `dive`
gives interactively. AUFS-style whiteout markers (`.wh.<name>`,
`.wh..wh..opq`) are detected and reported separately, not counted as
content — a layer that deletes a 500MB file doesn't get credited with 500MB
of "content," it gets a whiteout-count note, which is the correct read of
what that layer actually did to the image.

**Content size is uncompressed**, deliberately not the compressed
on-disk blob size `docker history`/`docker images` report — uncompressed
is the more useful number for "what's actually in this layer," but don't
expect it to match `docker images`' `SIZE` column exactly; they're
answering slightly different questions.

## Scope

Supports the **classic `docker save` tarball format** (top-level
`manifest.json` as a JSON array, `<layer-id>/layer.tar` paths) — what
`docker save` has produced since Docker existed, and what you get from the
command above with zero extra flags. **OCI image layout** (`index.json` +
content-addressed `blobs/sha256/...`, what `skopeo`/`crane`/`podman save
--format oci` produce) is a different manifest shape and isn't parsed yet
— a real v2, not attempted here to avoid half-supporting it. Layer blobs
*within* a supported tar are auto-detected as gzip-compressed or plain by
magic bytes, which does cover the compression `docker save` layers can use
either way.

## Status: built and verified against a real (hand-built, since no Docker daemon is available in this sandbox) image tarball

- **8 unit tests** (`cargo test --lib`): `manifest.json` parsing (including
  a dangling/untagged image with no `RepoTags` key at all, not just an
  empty array — `#[serde(default)]` is what makes that not a parse error);
  gzip magic-byte detection and round-trip decompression; whiteout
  detection separated from real file content, **with a direct assertion
  that a whiteout marker's presence doesn't inflate the reported content
  size**; largest-files sorted descending and correctly truncated to
  `--top`.
- **Full CLI run against a realistic hand-built tarball** — no Docker
  daemon available in this sandbox (confirmed: `docker ps` fails to reach
  `docker.sock`), so a `docker save`-shaped tarball was built directly with
  Python's `tarfile` module (manifest.json + two nested `layer.tar`
  entries, one of them containing a whiteout of a file from the layer
  before it) and run through the actual compiled binary reading from disk,
  not through the library API directly:
  ```
  layer 0 (layer1/layer.tar): 4.9 KiB in 2 files
         4.9 KiB  bin/myapp
            11 B  etc/config.yaml
  layer 1 (layer2/layer.tar): 19.5 KiB in 1 files, 1 whiteout(s)
        19.5 KiB  var/log/app.log
  ```
  every size and file count matches what was actually put in the fixture.
- **Gzip-compressed layer blob verified separately**: built a second
  tarball with a gzip-compressed `layer.tar` (the OCI-blob style of
  compression, layered inside the still-classic manifest format) and
  confirmed `layerscope` auto-detects and decompresses it correctly,
  reporting the right size.
- **Missing-manifest error path verified**: ran the binary against a tar
  with no `manifest.json` at all and confirmed the actual printed error
  names the real problem and points at this README, rather than a generic
  panic or an opaque parser error.

**Not done / deliberately deferred**: OCI image layout support (see
Scope), an interactive TUI (`dive`'s other headline feature — a
file-tree browser per layer — this v1 is a one-shot text report, the
natural v2 once the parsing core above is trusted).
