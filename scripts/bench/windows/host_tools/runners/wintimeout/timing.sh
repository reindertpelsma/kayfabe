#!/bin/bash
# Per run: async-preempt DISABLE_CHANNELS (refused or served), RUNLIST_PREEMPT_COMPLETE posts, the
# NV0073 snapshot controls, and the first twin free (the TDR teardown), with the kf3 maplog time.
cd /var/lib/kf-windows-20261005/wintimeout || exit 1
for r in 78 79 82 83 84; do
  f=run$r-qemu.log.gz
  echo "== run$r ($f)"
  zcat "$f" | awk '/maplog t=/ {match($0, /t=[0-9.]+/); t=substr($0, RSTART+2, RLENGTH-2)}
    /pRunlistPreemptEvent/ {print t, "async-preempt REFUSED"}
    /names channel .* of another client/ {print t, "DISABLE_CHANNELS REFUSED cross-client"}
    /async preempt: host preempt completed/ {print t, "async-preempt SERVED"}
    /act disable channels: .*bDisable=false/ {print t, "re-enable SERVED"}
    /RUNLIST_PREEMPT_COMPLETE posted/ {print t, "RUNLIST_PREEMPT_COMPLETE posted"}
    /act free: .*passthrough/ && !d {print t, "first twin free (TDR teardown)"; d=1}'
done
