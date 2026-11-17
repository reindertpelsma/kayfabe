#!/bin/bash
# the host baseline's runtime, the way provision_guest_apps.sh does it inside the guest: unpack the bundle to /opt/apps, run_apps.sh beside it, then setup_side.sh (same pins, same packages)
set -u
cd /workspace/kf-ci-src/scripts/apps
mkdir -p /opt/apps && tar xzf /workspace/apps/bundle.tgz -C /opt/apps && cp run_apps.sh /opt/apps/bundle/ && echo HOST_BUNDLE_IN=$(du -sh /opt/apps/bundle | cut -f1)
bash setup_side.sh 2>&1 | grep -E '^(SETUP_|IMPORT_OK|HF_MODEL|/dev)'
