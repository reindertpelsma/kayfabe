#!/bin/bash
cd /opt/nvkvm-steamos-two-container || exit 1
# Pinned to a SHA, not a branch name: the Dockerfile clones in a plain RUN, so
# docker caches that layer on the ARG value.  Rebuilding a moving branch under
# the same name silently recompiles the OLD commit -- it cost one full build
# cycle to notice.  A sha changes the ARG, which invalidates the layer, and
# makes the run reproducible besides.
export NVKVM_REF=5f20efa456cfd5bbc3bdca21dca331bbec62f349
export NVKVM_REPOSITORY=https://github.com/reindertpelsma/nvkvm-pv.git
docker compose build --progress plain 2>&1
echo "=== BUILD EXIT ${PIPESTATUS[0]} ==="
