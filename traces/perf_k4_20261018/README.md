# Q4 K4 GPU-side attribution

STATUS: RESEARCH IN PROGRESS, 2026-10-23. Candidate correctness remains PARTIAL/HOLD.

CODE READ ROUND_K4.md: explicit host return for a four-hour investigation, deadline11:00:58UTC and restore cutoff10:45:58UTC.

INFERRED plan.json: predeclared G1–G5 falsifiers, two fresh boots per arm, same K3 discipline; no product optimization.

MEASURED 2026-10-23, host-preflight.json: CUDA0, NVIDIA bound, GDM/Ubuntu/autologin and two
monitors confirmed before VM work; idle physical link Gen1x16, capability Gen4x16.
An idle downshift does not establish active downtraining. The original run4640
pre-launch failure is preserved in prelaunch-4640.json; no VM directory was
created. The missing staged eligibility receipt was supplied before run4644.

MEASURED 2026-10-23, analysis-controls.json:17 GPU-free evidence-integrity tests pass; four
mutants each fail two distinct assertions. Expected red24aa08d53 was an absent
new analyzer API/import, not a production runtime defect. The collectors and
proxy are own source; no product source or driver settings were changed.

INFERRED proxy-plan.json: the generated D3D11 grid compares real no-Present work
with identical rendering plus copy/present. CATIA/Creo use OpenGL; this proxy
cannot substitute for a no-present version of either viewset.
