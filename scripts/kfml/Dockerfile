# kfml test image (UNVERIFIED, never built). Build installs system packages and the venv; `kfml run` never does.
FROM nvidia/cuda:12.8.1-devel-ubuntu24.04
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv python3-dev gcc g++ git \
    && rm -rf /var/lib/apt/lists/*
COPY . /opt/kfml-src
WORKDIR /opt/kfml-src
ENV KF_ML_HOME=/opt/kfml-venv PATH=/opt/kfml-venv/bin:$PATH
RUN python3 -m kfml setup --venv /opt/kfml-venv --allow-unhashed
ENTRYPOINT ["python", "-m", "kfml"]
