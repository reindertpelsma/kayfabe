# docs/ — a map

**STATUS: LIVE, 2026-10-27.** The map of what is published and what is internal.

- **The user manual** is `docs/howto/` plus `docs/FAQ.md`. It is the only part of `docs/` that is
  published (https://kayfabe.dev; see `site/README.md`). `docs/howto/SUMMARY.md` is its navigation,
  and `docs/FAQ.md` is appended last by the site build.
- **Everything else under `docs/` is internal**: design documents (`design/`), status and handoff
  records, owner rulings, audits, references and archives. It is for contributors, is not part of the
  manual, and the site build refuses a manual page that links into it.
- **Agents and contributors start from `AGENTS.md`** at the repository root, which lists what to read
  first and the rules that bind the work.

A change that alters what a user sees or does updates `docs/howto/` in the same pull request. Design
notes never go into `docs/howto/`.
