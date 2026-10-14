#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
set -euo pipefail
cd "$(dirname "$0")"
observer_test_build=$(mktemp -d)
trap 'rm -rf "$observer_test_build"' EXIT
for compiler in cc clang; do
    "$compiler" -std=c11 -O1 -g -Wall -Wextra -Werror -fsanitize=address,undefined \
        -fno-sanitize-recover=all -Icore core/queue.c core/observer.c tests/core_test.c \
        -o "$observer_test_build/core-test"
    "$observer_test_build/core-test"
done
python3 tests/test_decode.py
