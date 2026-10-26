# kayfabe.dev

This directory contains the static site. `docs/howto/` is the user manual; its hand-written
`SUMMARY.md` controls navigation, with the unchanged `docs/FAQ.md` appended as FAQ. Other repository
docs are not published. A tag contributes a docs version only when it contains
`docs/howto/SUMMARY.md`. The initial manual is a small seed, not a complete installation guide.
The root README remains the source for landing copy and current project status.

## Build and check

Requires Python 3.12+, git and curl on Linux x86-64. No Node build, Cargo install or Python packages.
From the repository root:

```sh
TOOLS_DIR="$(mktemp -d)"
gh api repos/reindertpelsma/kayfabe/releases --paginate --slurp > /tmp/site-releases.json
python3 -m unittest discover -s site -p test_build.py
python3 site/build.py --tools-dir "$TOOLS_DIR" --releases-json /tmp/site-releases.json
python3 site/test_output.py
python3 site/report.py
rm -rf "$TOOLS_DIR"
```

Omit `--releases-json` for the no-published-release fallback. Keep the temporary tool directory to
repeat a build without network access: both cached archives are SHA-256 checked on every build.
`tools.json` pins Zola and mdBook release archives. Output is ignored at `site/public/`;
`site/build-report.json` records version/page counts and every broken local link (page, target,
reason). A rendering error, missing title or link into internal docs fails the build. Other missing
files/anchors are reported without failing. The output tests check SEO, search and version links.
The broker's workflow tests check the Pages permission boundary and dropdown behavior.

Master pins one toolchain; PR, master and tag builds render every eligible version and fail on any render failure.
Old HTML is always rebuilt; there are no per-version pins or frozen HTML caches.

## URLs and versions

- `/`: newest published `v*` release with assets and site content; master if none qualifies.
- `/staging/`: master landing, noindex, with no public navigation link to it.
- `/docs/master/`: current manual, labelled `master (unreleased)` and noindex.
- `/docs/<tag>/`: that tag's manual, when it has `docs/howto/SUMMARY.md`.
- `/docs/`: plain version list, search form, contributor link and redirect to the latest manual.
- `/docs/latest/`: redirect to the latest published release manual, or master before one exists.
- `/docs/versions.json`: dropdown data, ordered master then tags by descending git version order.

Latest means newest publication timestamp among non-draft releases with assets. A release lacking
the manual cannot become the latest docs; one lacking site data cannot become the root landing.
Search uses mdBook's own per-version index. The GET forms search the latest manual. The dropdown
keeps a chapter URL when HEAD finds it in the target version, otherwise opens that version's index.
Without JavaScript, the header and docs index retain plain version links.

Master's scripts, `book.toml`, theme, Zola config and tool pins build every ref. Tags supply only
`docs/howto/`, `docs/FAQ.md`, and site content/templates/static assets. This keeps old configuration
from accidentally changing the current renderer; refs are trusted and need no extra isolation.
All versions are rendered in PR builds too, including old tags. Root landing selection alone can be
overridden with the manual workflow's `root_ref=master` input.

Each page has metadata and an absolute canonical. Old docs canonicals point to the same chapter
under the latest published manual's real tag URL; master is self-canonical until a release exists.
The combined sitemap includes the root, docs index and latest release docs, excluding noindex
master/staging pages. Directory URLs end in `/`; mdBook chapter URLs retain their `.html` suffix.
`robots.txt` advertises the sitemap without blocking crawling of noindex pages. A 404 page is built.

## Workflow and publication

`site.yml` builds on PR changes to site/docs, master pushes, `v*` tag pushes, release publication,
manual dispatch, and a weekly fallback. A tag push makes its manual available immediately; publishing
release assets updates the root and latest docs. Non-PR builds explicitly check out master with full
tag history, even when the workflow was triggered at a tag, so they use the reviewed master tools.
PRs instead build the candidate merge commit, with only `contents: read`, and upload an artifact.
Only the separate master/tag deploy job has `pages: write` and `id-token: write`, in `github-pages`.
Concurrency uses `pages` without cancellation. All actions are SHA pinned.

CODE READ (2026-10-26): `release-build.yml` builds and attests artifacts; it has no release publishing
step. The `release: published` trigger covers manual/PAT publication. If a future job
publishes with `GITHUB_TOKEN`, append `gh workflow run site.yml` after its final asset upload and grant
`actions: write` only to that publishing job: token-generated release events do not start workflows.
The weekly fallback covers missed events; GitHub disables scheduled workflows in inactive public
repositories after 60 days. Dispatch manually or push to reactivate when needed. No settings or
secrets are changed by the build scripts.

CODE READ (2026-10-26): the existing heavy `ci.yml` has no docs/site path exclusion. Its stable, other
CPU jobs and broker jobs still run. The GPU gate uses two trusted-BASE decisions over complete PR
metadata: `docs/**` and `**/*.md` are skip paths, so docs-only and site-Markdown-only PRs skip GPU.
Site Python/HTML/CSS/TOML/assets, workflow changes and any `ci/vast-broker/**` change are relevant and
request the GPU gate. Unknown or truncated metadata requests a run; renames must have both paths in skip paths. This PR
does not change those policies.
