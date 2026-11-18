#!/bin/bash
# the apps stage's own provisioning (scripts/bench/box/apps_gate.sh with KF_APPS_PROVISION=1: build_bundle.sh, the guest image copy, provision_guest_apps.sh, the PROVISIONED marker)
# with a two-row list so the stage's matrix part is a few minutes, exactly as the payload's apps stage would provision.
cd /workspace/kf-ci-src
printf 'nvidia_smi\nvectorAdd\n' > /workspace/tiny_list.txt
env KF_APPS_PROVISION=1 KF_APPS_LIST=/workspace/tiny_list.txt bash scripts/bench/box/apps_gate.sh openguest_prov /root/prov middle
