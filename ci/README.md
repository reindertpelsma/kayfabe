# ci/

**STATUS: LIVE, 2026-11-23.** The Cloudflare Worker side of CI. The repository-side gates are in `scripts/ci/` and `.github/workflows/`; what each gate is, is in [`docs/CI_V3.md`](../docs/CI_V3.md). What is and is not deployed is stated in each component's own README and in `vast-broker/DEPLOY.md`; do not read this page as a deployment statement.

| Directory | What it is |
|---|---|
| [`vast-broker/`](vast-broker/README.md) | The Worker that rents Vast boxes for the GPU gates, with the tier logic, the ledger and the box payload lists. `DEPLOY.md` says what is deployed. |
| [`vast-reaper/`](vast-reaper/wrangler.toml) | The Worker that destroys boxes past their limits (`worker.js`; no README of its own, see `vast-broker/DEPLOY.md`). |
| [`vast-admin/`](vast-admin/README.md) | The owner-only admin page for the broker. |
| [`platform-api/`](platform-api/README.md) | The public platform API for status pages (design and code, see its STATUS). |
| [`r2-index/`](r2-index/README.md) | Folder listings for the public R2 buckets. |
| `requirements.toml` | Requirement sets per CI tier, read by the broker. |
