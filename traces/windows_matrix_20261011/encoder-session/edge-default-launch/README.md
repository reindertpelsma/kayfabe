# Ordinary Edge launch diagnostic

**STATUS: ANSWERED, 2026-10-11.** Run1260, product `a61af1c3`, trusted RTX4070,
host595.91.07, guest580.88, MPO disabled, normal profile, no Edge arguments.

Hypothesis: ordinary first launch can reproduce a recovery without video encoding.
Falsifier: the browser stays alive60s without guest events or GSP recovery.
Measured: PASS launch-only oracle64s, alive60s; zero driver/reset/WER records,
zero GSP cycles, no host Xids in the bounded kernel journal.

The screenshot shows Edge's first-run welcome screen. This measures survival
of that screen, not completed Edge setup, page navigation, video decoding or
GPU rendering correctness. It does not close the owner's observed black-window
recycle during run1161 or the independently recovered encoder runs1160/1162.
Exact click time and causal relationship remain unmeasured.

Files copied from the completed run on172.22.1.20. The source diagnostic inventory
is `../edge-default-launch-diagnostic.json`; original verdict/health/log/shot are
retained. This diagnostic is excluded from canonical matrix coverage counts.
