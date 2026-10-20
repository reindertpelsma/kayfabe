# Night N1 preregistration

STATUS: RESEARCH, 2026-10-20. Measurement and diagnosis only.

INFERRED hypothesis H1: two scripted GPU clients reproduce the owner's per-app
presentation silences. FALSIFIER, recorded before the run: either 12-minute A run
has no >=4s app silence, or their counts differ by more than a factor of three.
A/A agreement qualifies this load only, not every mixed workload. Terminal gaps
are labeled censored; watchdog/ETW evidence is needed to classify GPU versus
measurement/CPU stalls. Missing expected presents invalidates a run.

INFERRED H2: the defect was introduced after candidate-4. FALSIFIER: any verified
candidate-4 >=4s app silence under the same qualified load. B follows A/A first.

INFERRED H3: async disable/re-enable contributes. FALSIFIER: the otherwise matched
async-preempt-removed arm freezes. CODE READ: the historical ablation says this
flag is unnecessary for boot/idle Edge but explicitly ambiguous under load. This
permits the requested experiment; removal is not a fidelity fix.

INFERRED H4: an unfulfilled GPU semaphore acquire explains some stalls.
FALSIFIER: a valid stalled segment has no pending acquire, its awaited value is
already satisfied, or the entire private ring is consumed. G collects diagnostics
when an immutable binary and its report are available; do not infer semaphore
values from GP_GET alone.

CODE READ: D is excluded by the owner's cabling constraint. F equals the current
1920x1080 desktop as inspected before the first boot, so there is no distinct
1080p comparison unless later measurement finds a different baseline resolution.
C is available as a prebuilt candidate-5 binary; all comparisons require fresh
boots and at least two runs. A zero-freeze arm needs two further repeats and a
longer load before any absence claim. No product behavior will be changed.
