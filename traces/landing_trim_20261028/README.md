# Landing trim evidence

**STATUS: RESEARCH, 2026-10-28.** GPU-free editorial work, based on docs-pass-20261028.

MEASURED on 2026-10-28 UTC: `traces/landing_trim_20261028/benchmark.json` records twelve curl-fetched OSS/dev-tool pages, rendered in headless Chrome at 1440 by 900. `traces/landing_trim_20261028/copy-review.json` records the kayfabe before/after snapshots, current copy checks and mutation kills. These are page-content measurements, not hardware performance results.

The first document was supplied from the curl response; CSS and scripts loaded normally. Pages were scrolled to trigger lazy content, then returned to the top. Words were counted with DOM text ranges, computed visibility and horizontal viewport bounds. “First two screens” means word rectangles above y=1800. Counts are snapshots, not editorial targets; carousels, sticky elements and clipping can affect them. Raw third-party HTML and prose are not redistributed.

Sections mean a hero plus visible main h2 headings, with body fallback when there is no main. Every sampled project links to detail pages; several also have long home pages. lookingglass.io was excluded because it led to a parked-domain page; looking-glass.io is the project sampled. No page instructions were treated as commands.

Readability uses the same stdlib function as the site, in `site/claims_guard.py`. It counts prose paragraphs, list items and table cells, excluding navigation, headings, code blocks and comments. Sentences end at punctuation followed by whitespace or end of block; fragments count as one sentence. Decimal versions are one word. Syllables are vowel groups minus a silent trailing e, except le, with a minimum of one. Names, initials and numbers make Flesch approximate. Browser visibility and the static site parser also differ around CSS-only hidden text.

Final owner addendum 6 governs: average sentence length at most 20 words and every sentence at most 35 are hard checks for main-site pages. Honesty, the small hero check, no frame-rate figures and no em dashes remain hard checks. Flesch below 50 and depth limits are advisory. There is no word-count gate. Electron passes the sentence limits despite a Flesch estimate below 50; this is why the score is reported rather than used as a veto. The manual is outside these editorial limits.

## Content moves

| Former landing material | Destination |
|---|---|
| Full screenshot scene, application and per-die coverage, Windows limits | `/works-today/` |
| Rootless branch scope, two-VM limits, anti-cheat non-goal, missing integrations and build steps | `/works-today/` |
| Six alternatives, driver version differences, isolation model and licences | `/how-it-differs/` |
| CODE READ: Linux results and Windows status, with sources in `site/content/benchmarks.md` | `/benchmarks/` |
| Licence source, third-party notices and private vulnerability reporting | Existing `/about/`, `/security/` and linked manual pages |

Sources remain beside the moved statements as HTML comments. New pages use a shared detail template and appear in navigation and the sitemap. The README is unchanged. No new GPU or security result is claimed.
