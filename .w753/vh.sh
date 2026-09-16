#!/usr/bin/env bash
exec ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=15 -p 21284 root@ssh2.vast.ai "$@"
