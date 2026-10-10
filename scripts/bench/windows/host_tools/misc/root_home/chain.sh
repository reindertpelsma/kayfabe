#!/bin/bash
for i in $(seq 1 60); do grep -q "^\[.*\] DONE" /root/gs-test/main-mutated/run.log 2>/dev/null && break; sleep 10; done
cd /root/gs-test/repo && git checkout -q fix-guard && exec /root/gs_test.sh branch-mutated
