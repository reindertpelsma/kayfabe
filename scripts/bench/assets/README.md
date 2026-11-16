# Open test assets bundle

**STATUS: LIVE, 2026-11-16.** Standalone, stdlib-only builder for a bundle of redistributable test
assets (raster and raw photos, vector files, PDF, video, audio, 3D glTF models, and creative-application
project files under `project/`) that GPU creative and 3D application tests open. Measured on 2026-11-16: `fetch`, `verify`, `licenses` and `iso` ran end to
end from the source URLs on a box with network; the unit tests are GPU-free and offline.

Policy of 2026-11-16 (owner): every asset is pinned by sha256 and size, carries a `source_kind`, and keeps
its own licence. The three kinds and the synthetic section are described under *Source kinds* and
*Synthetic scale files* below.

Every asset keeps its own licence. The bundle only collects them: `test_assets.json` records, per file,
the licence id, the page where the licence was read, the exact attribution text, the source page, and
the pinned sha256 and size. Only CC0, public domain, CC-BY, CC-BY-SA (3.0 and 4.0, each entry flagged
`share_alike` and carrying its own attribution) and the
permissive software licences MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC and Zlib are admitted; the
validator rejects NC, ND, the GPL family and any other id. When a licence could not be established, the
asset was left out.

`project/` holds sample project files (layered raster, vector, page layout, motion graphics, video editing
and animation documents) taken from open-source repositories. Each is pinned to a commit, comes from a
repository whose own `LICENSE` file was read at that commit (`license_url`), and was written by the
repository owner for its tests. Files in such a repository that came from a third party are left out.
The MIT, BSD and Apache licences require the notice to travel with the file: `LICENSES.txt` carries the
copyright line and the licence page of each. Git LFS files are fetched from `media.githubusercontent.com`.

## Source kinds

`source_kind` is required on every entry (the manifest schema is `kayfabe.test-assets/2`).

| kind | meaning | what `fetch` does |
|---|---|---|
| `origin` | a stable, reliable source URL: a release host, a commit-pinned GitHub raw or media URL, a texture or raw-photo archive, a government server. No copy in public storage is needed. A Wikimedia host or a GitHub URL that is not pinned to a 40-hex commit is refused as an origin. | downloads `url`, checks the pin |
| `public-mirror` | open licence but an unstable origin (Wikimedia redirects, files re-uploaded in place). The file is **also** kept in the public bucket at `mirror_path` = `test-assets/<YYYYMMDD>/<name>`. | tries `url` first; if that fails or does not match the pin and `--mirror-base https://bundles.kayfabe.dev` is given, downloads `<base>/<mirror_path>`; the pin is checked on both |
| `private` | copyrighted or redistribution-prohibited file with real test value. Kept in a private object; `url` may be `null`, `fallback_url` is an optional official URL for maintainers. A `value` text is required: why the file is worth having and why no open equivalent works. Licence is `Proprietary`. | skipped; with `--include-private --private-dir DIR` the file must already be in `DIR` and its pin is checked (it is never downloaded or copied into the bundle directory) |

Private entries never enter the ISO, the `MANIFEST.json` copy or `LICENSES.txt`: `build`-side commands
(`licenses`, `iso`, `all`) list only the non-private entries, even when a private file sits in the directory.
`verify` handles all kinds: a private file is checked when present and only required with
`--require-private`. Classification on 2026-11-16: 7 `public-mirror` (every `commons.wikimedia.org`
file), 55 `origin` (commit-pinned GitHub raw and media, Poly Haven, raw.pixls.us, download.blender.org,
NASA NTRS), 0 `private`.

CC-BY-SA licences are admitted. An entry under one must have `share_alike: true` and its own
attribution; `LICENSES.txt` marks the file `[SHARE-ALIKE]` and states that derived outputs or evidence
that contain renders of a share-alike asset (screenshots, frames, recordings) must not be published. NC, ND
and unknown licences stay rejected.

## Synthetic scale files

The manifest section `synthetic` lists deterministic generated files for scale tests. They are not in the
ISO and not in `licenses` output unless `--with-synthetic` is given; the box that uses them generates them.

```
python3 scripts/bench/assets/build_test_assets.py synth --dest DIR [--only NAME] [--allow-unpinned]
python3 scripts/bench/assets/build_test_assets.py pin-synth --dir DIR      # maintainer: fill null pins
python3 scripts/bench/assets/build_test_assets.py verify --dir DIR --with-synthetic
```

| name | output | what | pin |
|---|---|---|---|
| `png_gradient_8k` | `synthetic/gradient_7680x4320.png` | 7680x4320 8-bit RGB PNG, about 100 MB | whole file |
| `tiff_16bit_big` | `synthetic/gradient_4096x4096_16bit.tif` | 4096x4096 uncompressed little-endian 16-bit RGB TIFF, about 100 MB | whole file |
| `wav_sine_sweep` | `synthetic/sine_sweep_48k_stereo_30s.wav` | 48 kHz 16-bit stereo, 30 s, 20 Hz to 20 kHz (left) and back (right) | whole file |
| `uhd_testsrc2_10s` | `synthetic/uhd_testsrc2_10s.mp4` | 3840x2160 25 fps 10 s H.264 from the pinned `ffmpeg` command in the manifest | hash of the first bytes only; skipped with a message when `ffmpeg` is not on `PATH` |

The three built-in generators are pure Python and stdlib only. Nothing that reaches the output uses libm
or a compressor: the sine table comes from an integer Taylor series, and the PNG pixel data is written as
stored (uncompressed) deflate blocks, so the bytes do not depend on the zlib build or the Python version.
An entry has either `sha256` + `size` (whole file) or `sha256_prefix` + `prefix_bytes` (generators whose
output can vary with the tool build or thread count). `synth` refuses an unpinned entry unless
`--allow-unpinned`, and deletes a generated file whose pin does not match. Measured on 2026-11-16: the
three built-ins generate in about 1.5 s in total and gave identical bytes on two runs. Not measured: the
`ffmpeg` entry (no `ffmpeg` on the maintainer host), so its pin is still null; whether its first bytes are
stable across thread counts is open.

## Rebuild from the source URLs

No object store is needed for `origin` entries. From the repo root:

```
python3 scripts/bench/assets/build_test_assets.py all --dest /var/tmp/test-assets --iso /var/tmp/test-assets.iso
```

or step by step:

```
python3 scripts/bench/assets/build_test_assets.py fetch    --dest DIR [--mirror-base https://bundles.kayfabe.dev]   # https only, no redirect to http
python3 scripts/bench/assets/build_test_assets.py verify   --dir  DIR   # sha256, size, no extra file
python3 scripts/bench/assets/build_test_assets.py licenses --dir  DIR   # LICENSES.txt, MANIFEST.json
python3 scripts/bench/assets/build_test_assets.py iso      --dir  DIR --out FILE.iso
```

`fetch` refuses a file whose sha256 or size differs from the manifest, a single file over 300 MB and
a bundle over 600 MB. A source that re-uploads a file in place (Wikimedia Commons does) makes `fetch`
fail on the pin rather than silently change the bundle; a maintainer then reviews the new file and
re-pins, which is why Wikimedia files are `public-mirror` entries. GitHub assets use a pinned commit,
never `main`.

The ISO is read-only ISO 9660 + Joliet (+ Rock Ridge when a tool builds it) with volume label `KFDATA`
and holds the assets, `MANIFEST.json` and `LICENSES.txt`. `iso` uses `genisoimage`, `mkisofs` or
`xorriso` when one is on `PATH`. With none, it prints a note and uses a built-in pure-Python writer
(`--engine python` forces it, `--engine tool` makes a missing tool an error). The Python image is
reproducible (`SOURCE_DATE_EPOCH`) and keeps the real file names through the Joliet tree.

## Maintainer: adding or changing an asset

1. Read the licence on the source's own licence page; write the page URL in `license_url`.
   Set `sha256` and `size` to `null`.
2. `fetch --dest DIR --allow-unpinned`, inspect the files, then `pin --dir DIR`. `pin` fills only nulls
   and exits with an error, writing nothing, if a file differs from an existing pin.
3. Run `python3 -I scripts/ci/test_test_assets.py`.

For a new `public-mirror` or `private` entry, upload the file to the bucket (or the private object) before
committing the entry; the manifest only holds the pin. Run the tests with
`python3 -I scripts/ci/test_test_assets.py`.

Downloaded files are untrusted data: this program hashes and copies them and never opens or executes
them. Do not commit downloaded files; only the manifest lives in the repository.
