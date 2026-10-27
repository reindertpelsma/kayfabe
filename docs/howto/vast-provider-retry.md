# Provider faults and one replacement

CODE READ from source at EXPECTED RED rev0913479ae plus this branch's replacement change:
the broker and BASE runner distinguish `PROVIDER_FAULT` from a kayfabe `FAIL`. A replacement
is requested once for an exact-instance frozen KVM-domain failure (including a provider that
labels the instance running), an explicitly missing `/dev/kvm`, or the existing driver
recovery's NOT_READY / GPU-unusable result (including the RTX3090 RmInitAdapter incident). A generic build/test
failure is still FAIL. A missing callback alone stays UNAVAILABLE, without blacklisting;
it does not prove whether the provider, Worker or bootstrap failed. Unmatched Xid or missing
safety evidence stays a stop and never becomes an automatic replacement.

The Worker destroys the old ledger-bound instance and confirms absence before replacement
admission. Its durable transaction checks the ended fault record and old nonce, keeps the
old cost receipt, and admits at most one different machine. The replacement gets a fresh box
capability/nonce and the original absolute hard/soft deadline. Both legs must fit the
original configured profile dollar allowance; actual replacement contract repricing is checked again. Daily dollars, rolling dollars, credit reserve and concurrent slots still apply.
Both legs retain their spend receipts but count as one attempt on its original UTC day;
CODE READ from Ledger.reserve in this branch: only its validated replacement bypasses
the new-attempt count and SHA-gap checks.
A different eligible host is preferred when available, without relaxing other offer filters.

The log hash chain and report/byte limits continue across both legs. The runner prints one
RENTAL line per instance. A replacement PASS makes the check pass; an exhausted provider
fault or budget-refused replacement reports PROVIDER_FAULT as an error, without labeling
kayfabe FAIL. The source fault and budget refusal remain visible. Machine exclusions persist
until owner review; their existing256-entry and ledger1000-row fail-closed bounds stay.

An already-open rental from before this update has no recorded per-run allowance and cannot
be automatically replaced. It still gets cleanup and a fault result. The coordinator deploys
the merged bundle and matching manifest; rerun a workflow whose trusted BASE manifest matches
that deployed bundle. Re-running an older workflow after a Worker update intentionally fails
the immutable bundle check.

The runner prints `provider fault: machine X, not a kayfabe verdict` with the validated machine id.
No extra driver-swap pre-check is added: the existing bounded driver recovery supplies the
incident evidence before product suites; repeating it would add work without proving a fresh GPU.

CODE READ from watch_delta and stage_grade in this branch: a failed provision command can
retain PROVIDER_FAULT only when its wrapper preserves the exact exit and positively reports
a readable, unchanged, clean Xid log. The product-pass path still rejects nonzero commands.
Missing/contradictory watcher proof, an Xid stop, an instrument failure or timeout stays a stop.
