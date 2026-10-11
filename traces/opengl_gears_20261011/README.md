# OpenGL gears investigation

**STATUS: RESEARCH, 2026-10-11.** Starts at integration `c1aa06b0`.

Run 643 uses the validated product binary `02afce90` with the GSP observer and corrected
sign-in harness. Hypothesis: boot-time static information or a previously missed control
reply diverges before the gears crash. Falsifier: all relevant model-owned static fields
and pre-crash replies agree with the working VFIO reference. A difference is a candidate,
not proof of causation; it needs a source-backed correction and a controlled run.

Pair replies by ordered function/client/object/command matches, not RPC sequence alone:
the existing reference uses sequence zero repeatedly. Observer polling timestamps do not
measure precise guest request-to-reply latency. Raw captures and closed-driver diagnosis
stay private; only behavior-level findings and open-source derivation enter this tree.
