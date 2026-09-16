#!/usr/bin/env bash
exec ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=15 -p 20902 root@ssh6.vast.ai "$@"
