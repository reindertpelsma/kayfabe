# Dependency producer only: the Vast consumer is the separately pinned KVM image.
FROM ubuntu:22.04@sha256:0a9887931b0e38af2e3487ed39e29436b0535c4f3f52e57745baaf79d1898956
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends python3 ca-certificates curl \
    && find /var/lib/apt/lists -depth -type f -delete
