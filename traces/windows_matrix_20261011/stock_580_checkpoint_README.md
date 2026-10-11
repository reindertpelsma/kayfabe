# Stock580.88 main-lane checkpoint

**STATUS: ANSWERED, 2026-10-11.**104/104 main-lane Windows-applicable rows
measured on owned Vast55385171, RTX4090, stock NVIDIA580.88. Latest rows:
103PASS,1TIMEOUT(GravityMarkVulkan). Zero recorded guest driver/reset/WER events
in the selected passing rows; native GSP cycles remain UNMEASURED throughout.
Eight overlay-policy rows are outside this main lane.

This is a combined checkpoint across explicitly listed harness revisions and
several batches, not one exact-revision sweep. TSV records the chosen latest
execution timestamp, source file and harness per row; all prior failures stay
in their original evidence directories. rebuild_stock_checkpoint.py regenerates
the table from committed data, rejects other GPU/driver setups, and never picks
a best result instead of the latest result. Diagnostic substitutions excluded.

The separate fresh50-row batch is50/50PASS at582d72dc. Corrected direct per-thread
stream738b5385 PASS7s, full independent CPU readback. Fresh frozen unchanged
BlackScholes738b5385 PASS closes the old retry's missing revision record.

Reference pass alone does not prove kayfabe pass. Candidate uses RTX4070, guest
580.88 and host595.91.07; stock reference is a different GPU and platform.
