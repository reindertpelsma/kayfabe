#!/bin/bash
# rent.sh <offer-id> [disk-gb=200] : the open-rows guest run's box (label kf-openguest-<ddhhmm>: the coordinator's cost guard caps that prefix at 5 h / 5 USD).
# First-boot key via --onstart-cmd (a VM's first boot can lack the account key). No token, key or credential goes on the box: only the PUBLIC key.
# The create output is filtered: only success and new_contract are printed (the response carries an instance_api_key that must never be shown).
set -eu
OFFER=$1; DISK=${2:-200}; LABEL="kf-openguest-$(date -u +%d%H%M)"
PUB=$(cat /root/.ssh/id_ed25519.pub)
vastai create instance "$OFFER" --image docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16 --disk "$DISK" --label "$LABEL" \
  --onstart-cmd "mkdir -p /root/.ssh && chmod 700 /root/.ssh && echo '$PUB' >> /root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys" --raw \
  | python3 -c 'import json,sys; d=json.loads(sys.stdin.read()); print("success",d.get("success"),"new_contract",d.get("new_contract"),"label","'"$LABEL"'")'
