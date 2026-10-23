MEASURED 2026-10-23, cohort.json: PARTIAL K4: CATIA+Creo completed in four primary measurement boots at714ec29d3: kayfabe4642/4649 and VFIO4648/4643, two boots per arm. All76 viewperf processes exited0, both viewsets completed per boot, and every arm exported/restored with zero153/4101/reboot/Budget/poison/new Xid. Three whole diagnostic runs pass; native4643 failed its optional post-collection display-mode verification. The original aggregate PASS was rejected by the independent proxy gate. Candidate correctness remains PARTIAL/HOLD; this is diagnostic evidence, not a merge clearance.

INFERRED 2026-10-23, cohort.json / proxy-results.json: no single GPU-side root cause is established. The strongest new control result is that the primary proxy contrast depends on desktop/window geometry. A slower kayfabe SM clock, active throttle condition or persistent physical-link downtrain is not observed in its selected windows. PCIe direction and guest accounting differ; those differences are not yet causal attribution.

CODE READ 2026-10-23, plan.json and controls/analyze_case.py: falsifiers preceded measurement. Same immutable parent,12GiB/8vCPU,FullHD viewsets; kayfabe9216MiB store/test broker60Hz/1920×1080 desktop versus VFIO12282MiB/physical primary3840×2160,virtual8640×2160. No product optimization or clock/timer setting changed. Only19 subtest interiors per boot enter the comparison, with the same3.601s exclusion at each edge. Clock membership covers the entire observed request/response bracket union; the earlier fixed3s model failure remains retained. Rolling polls are correlated; n2 fresh boots is the replication unit. Telemetry ranges compare boot means; proxy ranges compare phases, and memory peaks are separately labelled. None are confidence intervals.

MEASURED 2026-10-23, cohort.json: physical telemetry means [minimum–maximum boot mean]. Host NVML for kayfabe; guest NVML for physical VFIO. Each is200ms; query status/error fields are retained and unavailable values are never zero-filled.

| Workload / arm | GPU busy % | SM MHz | Memory MHz | Power W |
| --- | ---: | ---: | ---: | ---: |
| catia-07 / kayfabe | 76.2 [74.6–77.9] | 2854.4 [2853.9–2854.8] | 10251.0 [10251.0–10251.0] | 106.5 [104.6–108.3] |
| catia-07 / VFIO | 90.9 [90.0–91.8] | 2846.4 [2845.7–2847.1] | 10473.9 [10473.7–10474.0] | 127.6 [127.2–128.0] |
| creo-04 / kayfabe | 62.8 [62.1–63.5] | 2810.2 [2807.3–2813.0] | 10251.0 [10251.0–10251.0] | 82.1 [81.4–82.8] |
| creo-04 / VFIO | 59.2 [59.0–59.5] | 2461.8 [2453.2–2470.4] | 9231.6 [9198.0–9265.2] | 91.6 [91.5–91.7] |

INFERRED 2026-10-23, cohort.json G1: strict equal-clock falsifier is not met: memory clocks/P-states differ. Kayfabe remains P2 with zero active throttle masks; physical VFIO has P0/idle transitions and masks0/1. Both report200W power limits. The lower-kayfabe-SM/throttle explanation is weakened; measured state differences are not a demonstrated cause of the viewset gap.

MEASURED 2026-10-23, cohort.json: traffic rates below are API KB/s, not bytes per delivered frame. Kayfabe is Gen4×16 in every selected physical sample, with zero replay counters; native reaches Gen4×16, with brief lower generations in Creo. Virtual current-width query returns Unknown Error, not width0.

| Workload / arm | Memory-controller busy % | PCIe RX API KB/s | PCIe TX API KB/s |
| --- | ---: | ---: | ---: |
| catia-07 / kayfabe | 21.8 [21.1–22.6] | 247556 [246704–248409] | 272631 [266569–278692] |
| catia-07 / VFIO | 15.7 [15.5–16.0] | 517870 [508414–527326] | 57070 [55691–58449] |
| creo-04 / kayfabe | 16.5 [16.1–16.9] | 481329 [477242–485416] | 308422 [304720–312125] |
| creo-04 / VFIO | 20.9 [20.9–20.9] | 1274530 [1272174–1276885] | 199884 [198840–200927] |

INFERRED 2026-10-23, cohort.json G2: the equal-rate falsifier is not met. Native RX is higher while kayfabe TX is higher; memory-controller direction changes by viewset. This supports investigating placement/copies, not a claim of bus saturation, harmful page walks or a common all-traffic increase. Rates weight selected elapsed time; they are not per-frame GPU work and do not resolve bursts.

MEASURED 2026-10-23, proxy-results.json G3: completed render/Present-call iterations per second, not displayed new-content fps. Each mode has three3s phases/boot. Primary n2/arm; windowed n1/arm. Same1920×1080 RT,16draws/frame and8frames/batch; offscreen creates no window/swapchain and never calls Present. Both grid sizes are generated, not recovered CATIA/Creo draw sizes.

| Variant / vertices / arm | Boots | No Present mean [phase range] | Present mean [phase range] |
| --- | ---: | ---: | ---: |
| primary / 3000 / kayfabe | 2 | 4344.3 [4316.9–4379.6] | 62.4 [60.2–66.3] |
| primary / 3000 / VFIO | 2 | 3853.1 [3815.8–3886.9] | 3303.6 [3207.2–3385.1] |
| primary / 300000 / kayfabe | 2 | 2089.4 [2027.9–2135.7] | 62.0 [60.9–63.3] |
| primary / 300000 / VFIO | 2 | 1395.4 [1392.3–1400.5] | 1292.8 [1279.9–1305.5] |
| windowed / 3000 / kayfabe | 1 | 4325.0 [4230.5–4375.0] | 3570.1 [3433.7–3643.8] |
| windowed / 3000 / VFIO | 1 | 3830.0 [3808.1–3857.0] | 3207.9 [3002.4–3341.2] |

MEASURED 2026-10-23, cohort.json run4643 fullscreen-coverage control: MODE_CONTROL before_width=3840 before_height=2160 frequency=60 bits=32 orientation=0; MODE_CONTROL changed_rc=0 changed_verified=0 active_frequency=60. Applied-mode verification failed before any child proxy phase. Saved mode restoration was independently verified, and no extra GPU experiment followed. No fullscreen-control metric is available; its falsifier is untested. Original failed result and strict analysis rejection are retained in analysis-gate-failure.json. The PARTIAL observation ledger does not convert it to a clean whole-run PASS.

MEASURED 2026-10-23, proxy-results.json: all720 pixel checks passed, disjoint batches0/device-removed0. All six windowed Present phases have1280×720 visible,unminimized,foreground,uncloaked clients,zero occluded/other Present status. The primary did not log nonzero positive HRESULT/occlusion. INFERRED proxy-results.json G3: the near60 primary result cannot establish a general presentation-throughput deficit; windowing removes it in kayfabe. Generic D3D11 and unequal desktops cannot prove the OpenGL same-viewset hypothesis. The windowed proxy arm-ratio changes by 1.45% between modes, meeting its predeclared 10% qualified falsifier; n1/arm limits it. A real same-viewset no-present falsifier remains untested. GPU timestamp spans include CPU/queue gaps, not active kernel time.

CODE READ 2026-10-23, at 714ec29d3 broker.rs::BrokerSeat::frame: host memfd pages are registered for D2H and exported as udmabuf. MEASURED frame-backing-4642b.json: one retained8294400-byte broker frame has that exporter. This proves host backing for the captured frame, not every frame; GPU-copy-rung availability alone does not prove selected VRAM backing.

MEASURED 2026-10-23, cohort.json G4: all four whole-session supported NVAPI observers report eviction count/bytes delta0; the same API returns nonzero promotion counts and status0. Smaller/evolving observer budgets and commitment differ. Falsifier requiring equivalent budget behavior is not met. No observed NVAPI eviction weakens that specific mechanism, but does not prove absence of all WDDM paging. DXGI budget belongs to the observer, not viewperf. PDH adapter peaks below are selected-interior memory counters, not process budget limits; Linux/Windows FB/BAR1 accounting is not comparable resident occupancy.

| Workload / arm | Observer LOCAL mean budget GiB range | Adapter dedicated peak GiB range | Adapter shared peak GiB range |
| --- | ---: | ---: | ---: |
| catia-07 / kayfabe | 7.81–7.84 | 7.83–8.05 | 0.22–0.24 |
| catia-07 / VFIO | 10.74–10.74 | 3.28–4.31 | 0.22–0.25 |
| creo-04 / kayfabe | 7.91–7.91 | 2.90–2.93 | 0.38–0.40 |
| creo-04 / VFIO | 10.74–10.74 | 3.00–3.02 | 0.13–0.13 |

INFERRED 2026-10-23, cohort.json G5: scheduling/preemption remains unresolved. Physical encoder/decoder samples are zero; SM/memory differ. Guest PDH emits Copy rows for kayfabe versus3D+Copy for native, despite independent successful pixel oracles. Labels/accounting cannot establish physical engine/TSG equality, and omitted PDH zero rows do not prove absent engine types idle. Global GPU busy also includes VMM walker/display/copy work, so it cannot by itself rule out intermittent rendering starvation.

INFERRED 2026-10-23, cohort.json / proxy-results.json: ONE next experiment, after repairing/revalidating the failed display-mode oracle in a later host lease, is a matched-desktop CATIA+Creo comparison, two fresh boots per arm: one active1920×1080 guest desktop in both, same render resolution/driver/head/observers, and verified window coverage/flip mode. Predeclare the falsifier: if the candidate/native gap remains within the prior two-boot spread under matched geometry, reject geometry as its main explanation; if it changes outside spread, presentation context is implicated, not yet a product fix. Publish scores only via the stamped table. Do not implement RowsLog or a timer/polling change from these data.

MEASURED 2026-10-23, analysis-controls.json / clock-envelope-controls.json / transport-controls.json:38 GPU-free tests pass;12 Python mutants each lose at least two tests. Each of the four primary boots passes6 Windows identity checks;3 identity mutants each lose2 checks. Initial Import/API EXPECTED RED commits executed zero behavioral assertions. The later report-gate EXPECTED RED9772b2ae9 executes8 tests with5 behavioral failures, reproducing the outer PASS defect; the fixed report predicate rejects4643. The original host runner result is not rewritten. Original failed attempts remain in excluded-attempts.json; guards/timers were not weakened. No product source changed.

CODE READ 2026-10-23, plan.json: continuous native Windows dmon was not recorded; its two-sample support probes were saved and physical NVML/query200ms provides the requested fields throughout. Linux supplemental dmon is1s. No heavy GPU activity timeline, true same-viewset no-present or broker new-content tail distribution was measured. SPEC composites/ratios appear only in the stamped spec-comparison.md/CSV; these profiling runs are not official scores. M3 review, positive VMM CE missing-bind witness and outstanding exact-head qualification remain outside K4 clearance.

MEASURED 2026-10-23, host-yield.json / k4-host-return.json: host yielded 10:11:36 UTC with fresh CUDA0, NVIDIA/GDM/Ubuntu/autologin/two monitors, no own workers, Cargo target absent, lock released and quiet flag absent. The secondary full-journal 16MiB reader refused oversized text; its failure is retained. The unchanged lane Xid-only protocol under the same bound verified the complete 548-record sequence exactly. No GPU/VM work follows yield until explicit host return.
