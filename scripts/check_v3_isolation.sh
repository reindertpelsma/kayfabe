#!/usr/bin/env bash
# ★★★★★ v3 MAY NOT DEPEND ON THE OLD TREE — or archiving it achieves nothing.
#
# `[owner, 2026-09-21]` *"ensure that the dependency issues are solved so we don't have 60k lines
# of rot if we only use 1% of it."*
#
# ⊘ **The mechanism is dependencies, and it is silent.** The old tree is ~250 000 lines. If a v3
# crate declares `kayfabe-abi.workspace = true`, all **43 588** hand-maintained lines of it stay
# live — compiled, linked, and impossible to archive — no matter what any document says. One line
# in one `Cargo.toml` re-anchors the whole thing, and nothing about the build would look wrong.
#
# ⇒ The isolation has to be a GATE, not an intention. `THE_V3_PLAN.md` already states the target
# for the first crate — *"`kf-trap` … should be readable in one sitting, have **no dependencies but
# `kf-chip`**"* — and this enforces it.
#
# ⚠ It checks DECLARED dependencies, which is what determines whether a crate can be archived.
set -euo pipefail
cd "$(dirname "$0")/.."
# The prototype is now frozen; v3 consists of the actual kf-* workspace members.
# Parse TOML, including renamed, inherited and target-specific dependency edges.
exec python3 scripts/ci/dependencies.py
