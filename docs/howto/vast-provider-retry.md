# Provider faults and one replacement

CODE READ from source at EXPECTED RED rev0913479ae plus this branch's replacement change:
the broker and BASE runner distinguish `PROVIDER_FAULT` from a kayfabe `FAIL`. A replacement
is requested once for an exact-instance frozen KVM-domain failure (including a provider that
labels the instance running), an explicitly missing `/dev/kvm`, or the existing driver
recovery's `DRIVER_RC=8` with its failed bounded NVIDIA diagnostic. A generic build/test
failure is still FAIL. A missing callback alone stays UNAVAILABLE, without blacklisting;
it does not prove whether the provider, Worker or bootstrap failed. Unmatched Xid or missing
safety evidence stays a stop and never becomes an automatic replacement.

The Worker destroys the old ledger-bound instance and confirms absence before replacement
admission. Its durable transaction checks the ended fault record and old nonce, keeps the
old cost receipt, and admits at most one different machine. The replacement gets a fresh box
capability/nonce and the original absolute hard/soft deadline. Both legs must fit the
original dollar allowance; actual replacement contract repricing is checked again. The
daily count, daily dollars, rolling dollars, credit reserve and concurrent slots still apply.
The first real rental counts even if faulty; a replacement is a second real rental.

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
