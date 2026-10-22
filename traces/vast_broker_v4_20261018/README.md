CODE READ: 2026-10-22, V4 continues V3; the 2026-10-22 owner prompt and all three addenda apply, with addendum 3 superseding conflicting lifecycle requirements.
MEASURED: 2026-10-22, run v4-native-20261022 tested Worker source d0c3e8480 and the ab62c44c8 post-kill regression; native.json/native-runtime.json/post-kill-native.json contain the observations.
CODE READ: 2026-10-22, DONE—default 20-minute hard TTL (max 60), 10-minute provisioning, 5-minute PASS/FAIL/UNAVAILABLE grace (max 15), every-minute cron, no DO alarms.
CODE READ: 2026-10-22, DONE—own-run HMAC kf-box-kill, immediate owned deletion plus API absence, pre-result FAIL/STOPPED, post-result COMPLETE preserved, report-equivalent rate limits.
CODE READ: 2026-10-22, DONE—budget/prepaid reserve includes TTL plus one cron minute; default Worker reserve stays 8 USD and V4's local override was 5 USD.
CODE READ: 2026-10-22, DONE—bounded sanitized cursor logs, trusted live/final groups, structured summary and safe backtick fences; no artifact or proposed-code checkout.
MEASURED: 2026-10-22, v4-native-20261022 tests.json records baseline 537 green, standalone EXPECTED RED commits, and final 626/626 green with zero skips; bundle syntax passed.
MEASURED: 2026-10-22, v4-native-20261022 mutants.json records 32 V4 mutants plus 96 legacy cases, each with at least two named killers; mutation-table.md includes the coverage discoveries.
MEASURED: 2026-10-22, v4-native-20261022 native.json records mock PASS→kill, PASS→cron and FAIL→cron, 79 protocol requests and five real Wrangler scheduled-handler invocations; clocks/provider are synthetic.
MEASURED: 2026-10-22, v4-native-20261022 native-runtime.json records zero final uncaught/stream errors; ingress-native.json and the held-upload probe verify rejection and cron access without the upload entering the ledger.
MEASURED: 2026-10-22, v4-native-20261022 cpu-native-*.json records n=5 per route, response bytes and OS workerd CPU; this includes the local proxy/runtime and is not Cloudflare billed CPU.
MEASURED: 2026-10-22, v4-native-20261022 real-preflight.json records zero eligible offers at <=0.50 USD/hour, sufficient prepaid credit/reserve, zero real rentals and 0 USD new live spend.
CODE READ: 2026-10-22, PARTIAL—smoke.py provides real read-only preflight and native mock CI protocol; live orchestration/startup, actual box CLI, GPU output, real grace timing and hosted Actions/Cloudflare remain unverified.
CODE READ: 2026-10-22, Stage only with RENTING_ENABLED=false; the documented VM SSH-only launch conflict remains in the design STOP section. Public git excerpts are synthetic; GitHub Actions UI requires sign-in.
MEASURED: 2026-10-22, v4-native-20261022 cleanup.json records no own processes/assets/rules/tunnel/lock and removal of runtime secrets, native storage and temporary Wrangler; ANALYSIS.md explains remaining decisions.
