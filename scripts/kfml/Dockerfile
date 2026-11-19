# kfml test image (UNVERIFIED, never built). 2026-11-16: the wheel flavour is derived from the GPU and driver of the machine the
# container runs on, so the venv cannot be made at build time (no GPU there); the entrypoint runs `kfml setup` (flavour auto,
# hashed lock, --nvcc-if-needed) on first start into the venv directory, then the requested kfml command. Mount a volume at
# /opt/kfml-venv to keep the multi-GB venv between runs. `docker run --gpus all -v kfml-venv:/opt/kfml-venv kfml run --tier mid`.
FROM nvidia/cuda:12.8.1-devel-ubuntu24.04
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv python3-dev gcc g++ git \
    && rm -rf /var/lib/apt/lists/*
COPY . /opt/kfml-src
WORKDIR /opt/kfml-src
ENV KF_ML_HOME=/opt/kfml-venv
ENTRYPOINT ["/bin/sh", "-c", "python3 -m kfml setup --venv \"$KF_ML_HOME\" --nvcc-if-needed >&2 && exec \"$KF_ML_HOME/bin/python\" -m kfml \"$@\"", "kfml"]
