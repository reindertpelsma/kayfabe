# Tiny bootstrap migration

CODE READ: owner V5_ONSTART_AUTH contract replaces embedded transport with exact-SHA clone/checkout/exec; create env is a Docker flag string. The Worker embeds no run capability in its bootstrap. Renting without the path prefix refuses before OIDC or provider traffic.

MEASURED: run `v5-bootstrap-20261022` (`step1-green`, source derived from d9dbab1c8), 1007/1007 portable tests; the six previously red contract tests pass. Mutation results: `bootstrap_mutants.json`, each listed case has at least two named killers. No provider creation in this run.

CODE READ: tests migrated explicitly: callback-wiring, v4-sender, v4-lifecycle, v5-onstart, v5-soft-deadline and v5-structured-sender now read the actual repo-side shell/Python through a configuration-only test adapter. No sender implementation is copied into the adapter. The provider launch, prefix, lifecycle and checkpoint assertions now inspect create env or the tiny template. The two old optional-prefix rental tests now assert the owner's required-prefix refusal. Shared implicit fixtures gain a synthetic prefixed transport; explicit route-prefix tests retain raw URLs. Direct gateway and local-identity fixtures use explicit prefixes. Fourteen existing mutation sites moved to the repo scripts or the create env contract; their before/after guards remain explicit in mutation-anchors.json.
