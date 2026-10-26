# WK1 GATE-GREEN: default check selection

MEASURED 2026-10-26 at EXPECTED RED rev e7da61b65: two Node rental tests selected
machine100001 rather than cheaper eligible100002. Explicit die preference controls passed.
The initial workerd cases failed at the required path prefix and do not prove selection.
MEASURED 2026-10-26 at corrected EXPECTED RED rev fd4ae321d: both real workerd /rent
cases reached admission and selected100001 rather than100002. All outbound traffic was mocked.

CODE READ from source at rev fd4ae321d plus this branch's fix: default checks compare
eligible die representatives by effective hourly price. Country preference within each die,
explicit ordered preferences and sweep rotation retain their rules. DEFAULT_ORDER remains
the compatible protocol label. The public RENTAL line already reports the chosen offer's
machine/host/instance/GPU/price, so no wire change is needed.

MEASURED 2026-10-26: mutation evidence is in [mutations.jsonl](mutations.jsonl).
Run `python3 traces/wk1_green_selection_20261018/mutate.py` alone from repository root;
source is restored in finally. Never run it alongside another Worker test/build.

No deployment, Worker setting change or direct provider call was performed.

MEASURED 2026-10-26 on this branch after the fix: 2229 Node tests and51 real-workerd tests
passed, including the deterministic bundle-hash check. Both selection mutants were killed
by4 cases each in mutations.jsonl. Bundle hash is
dd92dab43725c07a3082438d4b77d883da1c8189fa05d038fca36745c836ac89.
