#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
set -euo pipefail
cd "$(dirname "$0")"
build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT
for compiler in cc clang; do
    "$compiler" -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -fno-sanitize-recover=all \
        queue.c tests/queue_test.c -o "$build/queue_test"
    "$build/queue_test"
done
python3 -m unittest discover -s tests -p 'test_*.py' -v
cc -std=c11 -Wall -Wextra -Werror -O2 -fPIC -shared queue.c -o "$build/libqueue.so"
python3 tests/oracle_replay.py "$build/libqueue.so"
