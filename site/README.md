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

Omit `--releases-json` for the no-published-release fallback: the landing's Download block then says
"No release yet: build from source". With a published release (non-draft, with assets) it shows that release, its
artifact links (only `github.com/reindertpelsma/kayfabe/releases/download/` URLs are accepted) and SHA-256 digests when
GitHub reports them. The build hands the templates one `data/site.json` (docs sections, versions, release).
To run the Zola render tests too, point `SITE_TOOLS_DIR` at a tools directory that holds Zola. Keep the temporary tool directory to
repeat a build without network access: both cached archives are SHA-256 checked on every build.
`tools.json` pins Zola and mdBook release archives. Output is ignored at `site/public/`;
`site/build-report.json` records version/page counts and every broken local link (page, target,
reason). A rendering error, missing title or link into internal docs fails the build. Other missing
files/anchors are reported without failing. The output tests check SEO (canonical, noindex, Open Graph and plain-text Twitter cards, sitemap), search, version
links, the landing's blocks, the manual's breadcrumbs, edit links and previous/next links, and that ordinary build output contains no external resources; deployment analytics is tested
separately with synthetic HTML.
The broker's workflow tests check the Pages permission boundary and dropdown behavior.

Master pins one toolchain; PR, master and tag builds render every eligible version and fail on any render failure.
Old HTML is always rebuilt; there are no per-version pins or frozen HTML caches.

## Portable copies

Use `python3 site/build.py --tools-dir "$TOOLS_DIR" --portable` to build a local copy.
Open `site/public/index.html` directly, or serve the whole output below any HTTP
prefix, such as `/x/y/`. Internal navigation, forms, assets, metadata URLs and
redirects use relative paths with explicit `index.html` directory targets. The
portable copy omits the production sitemap and robots file. The default build
keeps the existing Pages URLs and SEO metadata.

The version selector resolves its JSON relative to its script. Under HTTP it keeps
a chapter when that chapter exists in the selected version, otherwise opening the
version index. Under `file://`, where browsers can block fetch, the options included
in the document stay available and version changes open the selected index. The
no-JavaScript links work in either mode. No release packaging or publication job
is added by this build option.

Check portable output with `python3 site/check_portable.py`. This starts a loopback
server under `/x/y/`, checks all local HTML/CSS targets plus the selector/TOC/search
resources over HTTP, and verifies the corresponding `file://` targets exist. To
run the real two-version build fixture in the existing test suite, also set
`SITE_PORTABLE_TOOLS="$TOOLS_DIR"`. Use `python3 site/test_output.py` after a default
Pages build; those strict Pages assertions are intentionally unchanged.

## URLs and versions

- `/`: newest published `v*` release with assets and site content; master if none qualifies.
- `/staging/`: master landing, noindex, with no public navigation link to it.
- `/docs/master/`: current manual, labelled `master (unreleased)` and noindex.
- `/docs/<tag>/`: that tag's manual, when it has `docs/howto/SUMMARY.md`.
- `/docs/`: the documentation index: sections in `SUMMARY.md` order (lead-in chapters as "Start here", each
  `# Part` as written, the appended FAQ on its own) with the manual's own one-line descriptions, a search form,
  the evidence-label legend, the version list and the contributor link. Rendered by Zola from `docs_index.html`;
  a root landing from an older tag without that template keeps the plain version list and a redirect.
- `/about/`, `/security/`: short pages that point into the manual (the security page summarises and links, it does not
  replace `docs/howto/security.md`).
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

CODE READ (`.github/workflows/site.yml`): it evaluates every PR and builds changes to site/docs/workflow inputs, relevant
master pushes, `v*` tag pushes, release publication,
manual dispatch, and a weekly fallback. A tag push makes its manual available immediately; publishing
release assets updates the root and latest docs. Tag and other publication builds explicitly check out
master with full tag history, so they use the reviewed master tools.
PRs instead build the exact candidate head, with only `contents: read`, and upload an artifact.
Master pushes check out their exact SHA; publication events use master tooling.
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
metadata: `docs/**`, `site/**` and `**/*.md` are skip paths, including root README.md.
Workflow changes and any `ci/vast-broker/**` change request the GPU gate. Unknown or truncated
metadata requests a run; renames must have both paths in skip paths.

## Look and feel

<!-- src: .github/workflows/site.yml; site/build.py; site/deployment_slots.py; site/check_slots.mjs -->
Local, PR and fork output contains no publication slot, beacon or privacy text. The upstream
publication build adds one neutral head comment, using the layout or a verified byte splice for
other page families. Every page has a stable footer element. A pinned jsdom build check validates
that the slot is directly inside the head and that a dummy replacement adds only the expected nodes.
The protected deployment job validates its runtime HTML variable, accepting only canonical comments
and the approved script, then replaces that exact slot in bytes. It never parses pages, checks out
source or executes a downloaded helper. Valid input adds
the beacon and fixed privacy JavaScript from the workflow. That script adds one privacy sentence
only when the beacon element exists, waiting for the footer when needed. Empty, absent or invalid
input removes the slot and deploys without a beacon or privacy text; valid input with zero slots
fails. The actual variable is never stored in source or fixtures.
No frameworks or web fonts: system fonts, one small stylesheet (`static/style.css`) for the Zola
pages, `book-theme/kayfabe.css` layered on mdBook's CSS for the manual, and two optional scripts (a copy button on the
landing's command, and the version dropdown). Both follow `prefers-color-scheme` and `prefers-reduced-motion`. After
each mdBook render `build.py` adds a skip link, breadcrumbs, an "View and edit source on GitHub" link (master branch, the
page's source path), previous/next links and marks the manual's `[measured]`, `[inferred]` and `not documented yet`
text so gaps stay visible. Landing copy lives in `content/_index.md` front matter; each `src` field is written into the
page as an HTML comment, so every statement keeps its source. New claims need a source in the README or the manual.
The frontpage and the README must not contain the phrases listed in `claims_guard.py` ("near-native", "secure", "clean-room",
"anti-cheat compatible", "multi-tenant ready", "works with any GPU", hype words). `test_build.py` checks the sources and
`test_output.py` the rendered landing; a new banned phrase goes in that list with its reason.

## Proposed always-reported Site status

CODE READ 2026-10-27 (`.github/workflows/site.yml`, `site-status.yml`, `ci/vast-broker/site_policy.py`): PRs always get the custom `site` status. A read-only API decision skips an unrelated change; needed/uncertain changes build the exact PR head. The separate API-only reporter supports fork PRs without giving their code a status-writing token. Master deployment requires confirmed Site/docs/workflow changes, while existing publication events keep master tooling. See [status behavior](../docs/howto/site-status.md).
