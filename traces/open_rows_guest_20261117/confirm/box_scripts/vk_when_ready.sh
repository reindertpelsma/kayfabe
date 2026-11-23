#!/bin/bash
# wait for build_bundle.sh to have installed cmake and g++ (its open_dev_packages step) and released dpkg, then build the vkcts artifact
until command -v cmake >/dev/null && command -v g++ >/dev/null && command -v git >/dev/null && ! fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do sleep 10; done
cd /workspace/kf-ci-src && python3 scripts/apps/vkcts/vkcts.py check; python3 scripts/apps/vkcts/vkcts.py build --root /workspace/apps/vkcts
