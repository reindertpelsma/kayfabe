# WK1 GATE-GREEN: daily rental count ceiling

MEASURED 2026-10-26 at EXPECTED RED rev 0b0bf023b: proposed21/40 settings failed configuration
and two real-workerd DO reservations; upper-bound controls passed. Dollar guards also could
not configure40. The fix changes only the parser's maximum20 ->40, keeping the default4 and
all dollar/credit/concurrency controls. No deployment or deployed-var edit.

MEASURED 2026-10-26: mutation results are in [mutations.jsonl](mutations.jsonl); restore20
and widen400 must each fail at least2 tests. Run mutate.py alone from repository root.

Capacity recommendations and limits are in [settings](../../docs/howto/vast-budget-settings.md).

MEASURED 2026-10-26 on this branch: final2232 Node tests and51 real-workerd tests passed.
The two old SILENT configuration-refusal cases now use41/100 rather than21/40, matching the
new reviewed ceiling. The20-ceiling mutant causes7 failures and the400 mutant causes2.
Bundle b784ffb0fb357ffcfb51f09df246f072bfdda65dd06b4594cc2e0714bc1e0594.
