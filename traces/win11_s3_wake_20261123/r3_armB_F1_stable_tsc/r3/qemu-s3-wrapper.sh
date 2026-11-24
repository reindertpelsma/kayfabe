#!/bin/sh
exec '/workspace/bench/qemu-build-f1/qemu-system-x86_64' "$@" -global ICH9-LPC.disable_s3=0
