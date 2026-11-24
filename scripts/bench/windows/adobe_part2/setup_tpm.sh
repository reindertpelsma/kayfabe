#!/bin/bash
# Build libtpms 0.10.1 and swtpm 0.10.1 under /opt/tpm from the two tarballs in /workspace/p2 (sha256 pinned in propimage_apps.py / the run's pins).
# Same recipe as propimage_apps.build_swtpm. Log: /workspace/p2/tpm_build.log, last line TPM_BUILD_EXIT rc=N.
set -euo pipefail
exec > /workspace/p2/tpm_build.log 2>&1
trap 'echo "TPM_BUILD_EXIT rc=$?"' EXIT
echo "START $(date -u +%FT%TZ)"
cd /workspace/p2
echo "ebc24f3191d90f6cf0b4d4200cd876db4bd224b3c565708bbea0a82ee275e0fb  libtpms-0.10.1.tar.gz" | sha256sum -c -
echo "f8da11cadfed27e26d26c5f58a7b8f2d14d684e691927348906b5891f525c684  swtpm-0.10.1.tar.gz" | sha256sum -c -
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=1800 install -y --no-install-recommends build-essential automake autoconf libtool pkg-config libssl-dev libgmp-dev \
  libtasn1-6-dev libjson-glib-dev libgnutls28-dev libseccomp-dev libglib2.0-dev gawk socat expect gnutls-bin libfuse-dev libtool-bin net-tools \
  python3-cryptography tpm2-tools
export PKG_CONFIG_PATH=/opt/tpm/lib/pkgconfig LD_LIBRARY_PATH=/opt/tpm/lib
for spec in "libtpms-0.10.1 --with-openssl --with-tpm2" "swtpm-0.10.1 --with-openssl"; do
  set -- $spec; name=$1; shift
  rm -rf "src-$name"; mkdir "src-$name"; tar --strip-components=1 -xzf "$name.tar.gz" -C "src-$name"
  (cd "src-$name" && ./autogen.sh --prefix=/opt/tpm "$@" && make -j8 && make install)
done
/opt/tpm/bin/swtpm --version
echo "TPM_BUILD_OK"
