#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=3252a087
echo "PROD_DEFAULTS_CAMPAIGN_START=$(date -Is)"
trap 'rc=$?; echo "PROD_DEFAULTS_CAMPAIGN_EXIT=$rc at=$(date -Is)"' EXIT
bash /tmp/kf-prod-defaults-regression.sh > "$W/codex-prod-defaults-regression.log" 2>&1
bash /tmp/kf-prod-defaults-focus.sh > "$W/codex-prod-defaults-focus.log" 2>&1
python3 - "$W/codex-prod-defaults-focus-20261011-$REV" <<'CHECK'
import pathlib,json,sys
p=pathlib.Path(sys.argv[1]);rows={}
for line in (p/'win.res').read_text().splitlines():
 if line.startswith('APPRES '):
  d=dict(x.split('=',1) for x in line.split()[1:] if '=' in x); rows[d['app']]=d
assert set(rows)=={'glgears_wgl','vectorAdd_demo','cup8_ladder'},rows
assert all(d['verdict']=='PASS' and d['guest_tdr']=='0' and d['gsp_cycles']=='0' for d in rows.values()),rows
m=json.loads((p/'kernel-xids.json').read_text());assert m['query_exit']==0 and m['xid_records']==0,m
CHECK
# One bounded ablation, never re-run until a failure disappears. A failed
# experiment is retained and does not prevent the verified production session.
set +e
bash /tmp/kf-prod-caps-ablation.sh > "$W/codex-prod-caps-ablation.log" 2>&1
rc=$?
set -e
echo "CAPS_ABLATION_EXIT=$rc at=$(date -Is)"
bash /tmp/kf-prod-defaults-control.sh > "$W/codex-prod-defaults-control.log" 2>&1
python3 - "$W/codex-prod-defaults-control-20261011-$REV" <<'CONTROL'
import pathlib,json,sys
p=pathlib.Path(sys.argv[1]);rows=[]
for line in (p/'win.res').read_text().splitlines():
 if line.startswith('APPRES '): rows.append(dict(x.split('=',1) for x in line.split()[1:] if '=' in x))
assert len(rows)==3 and all(d['verdict']=='PASS' and d['guest_tdr']=='0' and d['gsp_cycles']=='0' for d in rows),rows
m=json.loads((p/'kernel-xids.json').read_text());assert m['query_exit']==0 and m['xid_records']==0,m
CONTROL
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
mkdir -p "$W/codex-prod-defaults-20261011"
printf '%s\n' "$REV" > "$W/codex-prod-defaults-20261011/manual-approved-revision"
bash /tmp/kf-prod-manual-1380.sh
