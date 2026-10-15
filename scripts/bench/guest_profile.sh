#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# The fat image's driver axis is independent of the host. Read a staging receipt as data;
# never source it, and never guess the guest version from nvidia-smi on the host.
kf_guest_driver_arg() {
    local image=$1 extra=${2:-} version= source= count=0 field key value
    local -a fields
    IFS=, read -r -a fields <<< "$extra"
    for field in "${fields[@]}"; do
        if [[ "$field" == guest-driver=* ]]; then
            version=${field#guest-driver=}; count=$((count + 1)); source=explicit
        fi
    done
    if [ "$count" = 0 ] && [ -r "$image.STAGED" ]; then
        source=receipt
        while IFS='=' read -r key value; do
            if [ "$key" = version ]; then version=$value; count=$((count + 1)); fi
        done < "$image.STAGED"
    fi
    if [ "$count" != 1 ] || [[ ! "$version" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
        echo "guest profile refused: $image needs one verified version in $image.STAGED or one explicit guest-driver= property" >&2
        return 2
    fi
    echo "KF_GUEST_PROFILE image=$image source=$source version=$version" >&2
    printf 'guest-driver=%s\n' "$version"
}
