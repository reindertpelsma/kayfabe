#!/bin/bash
# runs ON the box: stream a tar.gz of TEXT evidence to stdout (no images, binaries, archives, no URL file, no keys).
cd /
{
 find /workspace -maxdepth 1 -type f \( -name '*.log' -o -name '*.line' -o -name '*.marker' -o -name '*.pid' \) -size -20M 2>/dev/null
 find /workspace/apps/results -type f \( -name '*.res' -o -name '*.log' -o -name '*.txt' -o -name '*.tsv' -o -name '*.json' -o -name '*.csv' -o -name '*.dig' -o -name '*.base' \) -size -20M 2>/dev/null
 find /workspace/ogr_out /workspace/ogr -type f -size -5M 2>/dev/null
 find /root/prov -maxdepth 2 -type f \( -name '*.log' -o -name '*.txt' -o -name '*.run' -o -name '*.out' \) -size -20M 2>/dev/null
 find /workspace/bench -maxdepth 1 -type f \( -name '*.log' -o -name '*.out' -o -name '*.txt' \) -size -20M 2>/dev/null
 find /workspace/kf-ci-evidence -type f \( -name '*.json' -o -name '*.run' -o -name '*.log' -o -name '*.txt' \) -size -20M 2>/dev/null
} | sort -u | grep -v 'download-urls\|\.env$\|id_kf\|guest_key\|authorized_keys' | tar czf - -T - 2>/dev/null
