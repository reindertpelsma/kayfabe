#!/usr/bin/env bash
# ★★★ PROVISION A CUDA-CONTAINER BOX FOR THE BARE-METAL RAW CLIENT.
#
# > Owner, 2026-09-20: *"you may also use vast cuda container"* — for bare metal there is no
# > guest, so a KVM template buys nothing (memory: `rent_a_kvm_template_unless_the_test_is_bare
# > _metal`). A CUDA container is cheaper, boots faster, and leaves the KVM bench free for the
# > guest lane so the two run in PARALLEL instead of contending for one GPU.
#
# ⊘ THE BOX PULLS; NOTHING IS PUSHED. Standing constraint: *"ALL code stays LOCAL and boxes
# pull; never copy executables back; no secrets on a box."* The repo is public, so a clone is
# the whole transfer — no key, no scp, no binary in either direction.
#
# usage: provision_bare_metal.sh <ssh-host> <ssh-port>
set -uo pipefail
H=${1:?ssh host}; P=${2:?ssh port}
S="ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o ConnectTimeout=20 -p $P root@$H"

echo "== driver and card (the two facts every result must be read against)"
$S 'nvidia-smi --query-gpu=driver_version,name --format=csv,noheader; ls /dev/nvidia* 2>/dev/null | tr "\n" " "; echo' || exit 2

echo "== toolchain"
# ⊘ `x86_64-unknown-linux-musl` is NOT optional: `kayfabe-isolate-host`'s build.rs performs a
# NESTED build of the isolate image for that triple and panics by name without it
# (`build.rs:303`). `[measured w814d]` a fresh CUDA container has the host triple only, so the
# first provision failed here — and the error told us exactly what to add, which is the only
# reason this cost minutes instead of an hour.
$S 'command -v cargo >/dev/null 2>&1 || { apt-get update -qq && apt-get install -y -qq curl build-essential pkg-config git >/dev/null 2>&1; curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal >/dev/null 2>&1; }; export PATH=$HOME/.cargo/bin:$PATH; rustup target add x86_64-unknown-linux-musl >/dev/null 2>&1; cargo --version; rustup target list --installed | tr "\n" " "; echo' || exit 2

echo "== pull the tree (public repo; nothing is pushed to the box)"
$S 'export PATH=$HOME/.cargo/bin:$PATH
    if [ -d /root/kayfabe/.git ]; then cd /root/kayfabe && git fetch -q origin; else
        git clone -q https://github.com/reindertpelsma/kayfabe.git /root/kayfabe && cd /root/kayfabe; fi
    git checkout -q w749-fable-legb && git reset -q --hard origin/w749-fable-legb && git log --oneline -1' || exit 2

echo "== build the raw client (release, as the guest lane runs it)"
$S 'export PATH=$HOME/.cargo/bin:$PATH; cd /root/kayfabe && cargo build -q --release -p kayfabe-isolate-host --bin kayfabe-rm-ladder 2>&1 | tail -5; ls -la target/release/kayfabe-rm-ladder' || exit 2

echo "== SANITY: the client must be able to open the driver at all"
# ⊘ A one-arm smoke BEFORE the sweep. Without it a box with no /dev/nvidiactl produces 30
# identical failures and they read as 30 findings instead of one environment fact.
$S 'cd /root/kayfabe && timeout 120 ./target/release/kayfabe-rm-ladder --timer 2>&1 | tail -3; echo "SMOKE_RC=$?"'
