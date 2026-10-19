# Windows boot and acquisition inventory

MEASURED: every P1 Windows boot uses the same persistent throwaway overlay, isolated from the owner's original writable disk. All flags below add to the same production profile and lifecycle counters. NSI means KF3_DIAG_NSI_TRACE=1 with an arm-specific dump path; GET means KF3_DIAG_GP_GET_FULL_ONLY=1. No mirror-OFF or VFIO comparison was run. The immutable product revision and actual full environment are archived per arm under the host lane directory.

| Run | Arm | Revision | Broker | Extra measurement setting | Completed comparison captures | Disposition |
|---|---|---|---|---|---:|---|
|41001|mirror-b1|0d7ba743|new c268 meter|none|0|ROM setup refused before guest startup; no performance number|
|41002|mirror-b2|0d7ba743|new c268 meter|none|0|boot TDR/reset observed; early probes excluded from comparative arms; broker causality unproved|
|41003|mirror-c6-quiet|0d7ba743|unchanged cursor6|none|0|quiet boot/health confirmation; no Steel Nomad comparison sample|
|41004|mirror-c6-b4 / mb4|0d7ba743|unchanged cursor6|none|3|initial mirror control,15 pure queue probes|
|41005|p1-diag-on-1 / p1on1|41a8beb0|unchanged cursor6|NSI|5|diagnostic control,15 valid pure probes; incomplete COMPUTE terminal-marker invocation retained separately|
|41006|mirror-b5|0d7ba743|unchanged cursor6|none|2|mirror control extension,15 pure probes; both long content gaps retained|
|41007|pacer20|41a8beb0|unchanged cursor6|NSI + interval20us|5|16 pure probes; first calibration rejected, unchanged labelled quiet calibration passed|
|41008|pacer50|41a8beb0|unchanged cursor6|NSI + interval50us|5|16 pure probes; no beyond-spread content gain|
|41009|get-full1|41a8beb0|unchanged cursor6|NSI + GET|0|initial boot healthy; launch and read-only dispatch timeout; further performance STOP|

MEASURED: RESULTS.md/RESULTS.json enumerate all20 completed Steel Nomad captures, including original control order and every headline metric. MICRO_RUNS.md enumerates77 complete pure tiny invocations. W2_EDGE_RESULTS.json summarizes the three final3000-iteration diagnostic DIRECT traces, separately from pure performance runs. Failed/partial records and setup errors remain preserved in HARDWARE_PROGRESS.md and the private raw archive; no missing terminal marker is silently reconstructed from its CSV.

MEASURED: additional acquisition records include an initial C-ABI/irqfd observer identity rejection before the public kf3_irq_fd observer was installed, an early overlap scheduling mistake, one incomplete early COPY probe with a successful labelled quiet repeat, the incomplete diagnostic COMPUTE invocation with its labelled quiet repeat, and the20us clock calibration rejection with its unchanged quiet repeat. These are acquisition outcomes, not extra accepted comparative benchmark samples. Exact tag/raw trace reports remain under the host lane directory. The initial new-meter-broker boot reset and GET-mode failure remain unexplained; clean later arms do not erase either failure.
