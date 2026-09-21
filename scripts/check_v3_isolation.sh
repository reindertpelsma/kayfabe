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
set -uo pipefail
cd "$(dirname "$0")/.."

# The v3 crates. ⊘ An explicit list, because "new" is not a property cargo knows.
V3_CRATES="kayfabe-doorbell"
# What a v3 crate is allowed to depend on: other v3 crates, and nothing else from this tree.
ALLOWED="$V3_CRATES"

bad=0
for c in $V3_CRATES; do
    f="crates/$c/Cargo.toml"
    [ -f "$f" ] || { echo "⊘ $c: no Cargo.toml"; bad=$((bad+1)); continue; }
    deps=$(sed -n '/^\[dependencies\]/,/^\[/p' "$f" | grep -oE '^kayfabe-[a-z-]+' || true)
    for d in $deps; do
        case " $ALLOWED " in
            *" $d "*) ;;
            *)
                n=$(find "crates/$d/src" -name '*.rs' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
                echo "⊘ $c depends on OLD crate $d (${n:-?} lines) — that crate can never be archived"
                bad=$((bad+1))
                ;;
        esac
    done
    echo "✔ $c: $(echo "$deps" | grep -c . 2>/dev/null || echo 0) in-tree deps"
done
echo "V3_ISOLATION crates=$(echo $V3_CRATES | wc -w) violations=$bad"
[ "$bad" -eq 0 ]
