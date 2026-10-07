#!/usr/bin/env bash
# Print vast.ai balance + runway. Exits 1 if runway < 24h.
# WHY: credit exhaustion DESTROYS instances. From the driver's seat that is
# indistinguishable from a crashed lane -- instance gone, no output, no
# terminator line. Same class as every "absent artefact reads as favourable"
# trap in CLAUDE.md. Make it a checked state, not an inference.
set -uo pipefail
bal=$(timeout 90 vastai show user --raw 2>/dev/null \
      | python3 -c 'import sys,json;print(json.load(sys.stdin).get("credit",0))') || exit 2
[ -n "${bal:-}" ] || { echo "VAST_BALANCE=UNKNOWN (query failed)"; exit 2; }
burn=$(timeout 90 vastai show instances --raw 2>/dev/null \
      | python3 -c 'import sys,json;print(sum((i.get("dph_total") or 0) for i in json.load(sys.stdin)))') || exit 2
echo "VAST_BALANCE=\$${bal}  VAST_BURN=\$${burn}/hr"
python3 - "$bal" "$burn" <<'PY'
import sys
b,h=float(sys.argv[1]),float(sys.argv[2])
if h<=0: print("VAST_RUNWAY=inf (no instances running)"); sys.exit(0)
r=b/h; print("VAST_RUNWAY=%.0fh"%r)
if r<24: print("!! VAST LOW CREDIT: under 24h. Instances are DESTROYED at zero, not paused."); sys.exit(1)
PY
