#!/bin/bash
# kfpush.sh LOCAL GUESTPATH : write a (small, < ~200 KB) file into the guest through the kfagent
B64=$(base64 -w0 "$1"); G=$2
timeout 90 python3 /data/kfagent_server.py run "New-Item -ItemType Directory -Force (Split-Path \"$G\") | Out-Null; [IO.File]::WriteAllBytes(\"$G\", [Convert]::FromBase64String(\"$B64\")); \"pushed $G\"" 60 | tail -1
