# Open test assets bundle

**STATUS: LIVE, 2026-11-16.** Standalone, stdlib-only builder for a bundle of redistributable test
assets (raster and raw photos, vector files, PDF, video, audio, 3D glTF models) that GPU creative and
3D application tests open. Measured on 2026-11-16: `fetch`, `verify`, `licenses` and `iso` ran end to
end from the source URLs on a box with network; the unit tests are GPU-free and offline.

Every asset keeps its own licence. The bundle only collects them: `test_assets.json` records, per file,
the licence id, the page where the licence was read, the exact attribution text, the source page, and
the pinned sha256 and size. Only CC0, public domain, CC-BY and (flagged `share_alike`) CC-BY-SA are
admitted; the validator rejects NC, ND and any other id. When a licence could not be verified, the
asset was left out.

## Rebuild from the source URLs

No object store is needed. From the repo root:

```
python3 scripts/bench/assets/build_test_assets.py all --dest /var/tmp/test-assets --iso /var/tmp/test-assets.iso
```

or step by step:

```
python3 scripts/bench/assets/build_test_assets.py fetch    --dest DIR   # https only, no redirect to http
python3 scripts/bench/assets/build_test_assets.py verify   --dir  DIR   # sha256, size, no extra file
python3 scripts/bench/assets/build_test_assets.py licenses --dir  DIR   # LICENSES.txt, MANIFEST.json
python3 scripts/bench/assets/build_test_assets.py iso      --dir  DIR --out FILE.iso
```

`fetch` refuses a file whose sha256 or size differs from the manifest, a single file over 300 MB and
a bundle over 600 MB. A source that re-uploads a file in place (Wikimedia Commons does) makes `fetch`
fail on the pin rather than silently change the bundle; a maintainer then reviews the new file and
re-pins. GitHub assets use a pinned commit, never `main`.

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
3. Run `python3 -m unittest scripts.ci.test_test_assets`.

Downloaded files are untrusted data: this program hashes and copies them and never opens or executes
them. Do not commit downloaded files; only the manifest lives in the repository.
