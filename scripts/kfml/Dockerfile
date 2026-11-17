# kfml test image (UNVERIFIED, never built). Build installs system packages and the venv; `kfml run` never does.
# [2026-11-17] pinned by registry digest (audit row 20, scripts/ci/trust_manifest.json `kfml-base-image`): the digest is the manifest list that
# Docker Hub served for this tag on 2026-11-17 (MEASURED, `Docker-Content-Digest` of registry-1.docker.io); a tag can be re-pushed, a digest cannot.
FROM nvidia/cuda:12.8.1-devel-ubuntu24.04@sha256:520292dbb4f755fd360766059e62956e9379485d9e073bbd2f6e3c20c270ed66
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv python3-dev gcc g++ git \
    && rm -rf /var/lib/apt/lists/*
COPY . /opt/kfml-src
WORKDIR /opt/kfml-src
ENV KF_ML_HOME=/opt/kfml-venv PATH=/opt/kfml-venv/bin:$PATH
RUN python3 -m kfml setup --venv /opt/kfml-venv --allow-unhashed
ENTRYPOINT ["python", "-m", "kfml"]
