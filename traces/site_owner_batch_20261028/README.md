# Site owner batch: link-pattern review

**STATUS: RESEARCH, 2026-10-28.** Editorial evidence only; no product or hardware result.

MEASURED in `link-study.json`, 2026-10-28: eleven primary pages fetched with curl, with SHA-256 identities and link counts per h2 section and per 100 static-text words. The earlier rendered snapshots in `traces/landing_trim_20261028/benchmark.json` were reviewed first. This follow-up excludes script, style, nav, header and footer elements; custom navigation markup, CSS visibility and client-rendered prose can still affect counts. Immich's static response has almost no body prose, so its rate is not comparable with the others. No third-party HTML is redistributed.

## Ten-line pattern summary

1. Tailscale: 10 content links, 0.78 per 100 static words; section counts 5/3/0/2; action and case-study links, no citation apparatus.
2. Asahi: 9 links, 7.09 per 100 words; sections 1/8; short intro followed by install, support and documentation links.
3. Zed: 59 links, 2.58 per 100 words; section counts 3/0/12/9/22/7/0/1/1/1/1/2; feature links and buttons; status says public beta.
4. Immich: 16 links in 28 static words, one section; app, server and docs links; dynamic prose prevents a meaningful density comparison.
5. Syncthing: 9 links, 2.44 per 100 words; sections 1/8; links sit on claim words such as security and specification.
6. Caddy: 17 links, 0.75 per 100 words; sections 3/1/0/0/1/3/1/0/4/1/3/0/0; claim links plus three visible research URLs in expert quotes.
7. Looking Glass: 8 links, 4.85 per 100 words; sections 3/3/0/2; short introduction and resource actions, no citation apparatus.
8. WireGuard, Podman and Proxmox: 2.70/5.16/4.74 links per 100 words; section counts remain in the JSON; docs, feature and action links dominate.
9. Tailscale's WireGuard comparison uses topic sections on configuration, connectivity, security, performance and administration; no table or footnote citations were observed.
10. Follow the short-intro/detail-link pattern, attach useful links to the claim words, and state limits plainly; keep our evidence in HTML comments and the central status list.

CODE READ: except for Caddy's quoted research links, the inspected main copy uses no visible evidence-citation apparatus. Status wording is sparse: Zed uses public beta and WireGuard links to Known Limitations; most homepages send readers to details. These are observations of these captures, not a universal OSS-site rule.

## Scope and verification

CODE READ: `docs/FAQ.md` and `docs/howto/credits.md` hold project-specific detail; the main comparison groups technologies and gives Open-IOV one sentence under NVIDIA vGPU. Caraxes remains in the credits and the single similar-project FAQ answer. Source URLs sit beside the edited claims. The existing landing already follows the short-intro/action-link pattern and retains its source comments.

The Caraxes quickstart's module-options command is in guest preparation, so the correction calls them guest-driver options. Its unprivileged-client description is in the quickstart, not the README. Open-IOV's overview says the distribution NVIDIA module is unbound, while its install guide also describes a path using the distribution driver; the detailed credits record the difference without claiming host coexistence.

No product or build behaviour changes. Existing rendered-copy expectations were updated for the requested headings; no new prose-mirroring tests, EXPECTED RED commit or product mutants apply.
