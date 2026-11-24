#!/bin/sh
exec '/workspace/bench/kf3-bins/1052e2f9/qemu-system-x86_64' "$@" -global ICH9-LPC.disable_s3=0
