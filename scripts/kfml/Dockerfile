# kfml test image (UNVERIFIED, never built). 2026-11-16: the wheel flavour is derived from the GPU and driver of the machine the
# container runs on, so the venv cannot be made at build time (no GPU there); the entrypoint runs `kfml setup` (flavour auto,
# hashed lock, --nvcc-if-needed) on first start into the venv directory, then the requested kfml command. Mount a volume at
# /opt/kfml-venv to keep the multi-GB venv between runs. `docker run --gpus all -v kfml-venv:/opt/kfml-venv kfml run --tier mid`.
# [2026-11-17] pinned by registry digest (audit row 20, scripts/ci/trust_manifest.json `kfml-base-image`): the digest is the manifest list that
# Docker Hub served for this tag on 2026-11-17 (MEASURED, `Docker-Content-Digest` of registry-1.docker.io); a tag can be re-pushed, a digest cannot.
FROM nvidia/cuda:12.8.1-devel-ubuntu24.04@sha256:520292dbb4f755fd360766059e62956e9379485d9e073bbd2f6e3c20c270ed66
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv python3-dev gcc g++ git \
    && rm -rf /var/lib/apt/lists/*
COPY . /opt/kfml-src
WORKDIR /opt/kfml-src
ENV KF_ML_HOME=/opt/kfml-venv
ENTRYPOINT ["/bin/sh", "-c", "python3 -m kfml setup --venv \"$KF_ML_HOME\" --nvcc-if-needed >&2 && exec \"$KF_ML_HOME/bin/python\" -m kfml \"$@\"", "kfml"]
