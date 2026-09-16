#!/usr/bin/env bash
# ⚠ DIRECT IP, not ssh2.vast.ai:21284 — the vast SSH PROXY refused every connection for
# 25 minutes on two consecutive boxes while the box itself was up and answering on its own
# address. `actual_status` flipping to `running` did not change the proxy's answer.
exec ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=20 -p 54698 root@184.144.255.144 "$@"
