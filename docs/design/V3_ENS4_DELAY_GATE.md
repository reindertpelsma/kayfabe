# Apply-delay intervention and Linux latency gate

**STATUS: LIVE diagnostic implementation, 2026-10-26; GPU-free validation only.** Hardware is unrun. The per-boot completion-observation scalar is implemented with explicit missing/proxy coverage; a raw GET alone is never a completion.

## Intervention

CODE READ, `kf-mem::applydelay` and `VaManager::on_walk_ready`: `KF3_DIAG_APPLY_DELAY_MS` is a host-operator-only unsigned decimal dose0..5000ms, default0/OFF. `KF3_DIAG_APPLY_DELAY_MIN_RUNS` is1..1000000, default1000 attempted diff runs. Invalid configuration names the error once and disables injection. A guest chooses neither value. Every qualifying apply sleeps once on its existing VA-manager caller after begin recording, before the real map/unmap apply. The ordinary global walk gate remains held exactly as for a slow apply. No thread is spawned, no vCPU sleeps, no separate gate is added and no GPU is paused. No gate/timeout policy is repaired or relaxed.

CODE READ, `gatelog/ens3.rs`: begin-apply metadata records the requested dose before sleeping, so an in-flight first153 capture can show it. The final `injected_wall_ns` is actual elapsed sleep/scheduling time inside `wall_ns`, not an additive phase. A pending apply still says `finished=false`; its final wall costs are unavailable. Thread CPU time excludes most sleeping, whereas CPU-residual wall time includes it. The optional ENS3 mode also emits qualifying `ENS4 APPLY` lines, even at dose0, for Linux coverage validation. Status includes requested dose/threshold, number of delayed applies and total injected wall time.

CODE READ, `mem/ens4_tests.rs`: the real Inbox global predicate remains busy during fake-sleeper execution for both an in-flight walk and a queued global split. These are virtual-time contract tests, not a hardware stall or a Windows watchdog model.

## Deliberate failure and Linux arm

The new `kf-ens4-gate` executable has no default GPU action. `--sim` uses virtual time; only explicit `--gpu --csv FILE` opens the ordinary unprivileged RM connection. No GPU mode was run here.

```sh
CARGO_TARGET_DIR=<shared-target> nice -n 19 cargo build --release -j2 \
  -p kf-harness --bin kf-ens4-gate
<shared-target>/release/kf-ens4-gate --sim --apply-us 0
<shared-target>/release/kf-ens4-gate --sim --apply-us 77000
```

MEASURED in the 2026-10-26 GPU-free run, `traces/ens4_20261018/deliberate-gate.json`: the no-hold model exits0/PASS; the conditional77ms apply exits1/FAIL, with a synthetic100µs clean p99.9 and400µs bound. Its packet arrives1ms into that apply and completes at77.1ms, for76.1ms end-to-end. This demonstrates the detector and the global-hold ordering; it does not establish actual Linux/Windows packet latency or a TDR. ENS2 interval means are not individual apply traces. The deliberate-red arm remains red: no production gate narrowing or latency fix has been made.

CODE READ, Linux mode: the existing CeRig submits small16-byte copies on one owned CE channel and checks its actual CE release and WFI fence. Neither an IRQ nor a raw GET/REFERENCE substitutes for the fence. Latency includes packet preparation, doorbell and real completion observation. After1024 warmups,10000 clean samples freeze nearest-rank p99.9 and bound=4×p99.9; k=4 is a preregistered jitter margin, and the actual numeric bound comes from calibration. Any missing receipt or over-bound packet fails, including an early failure followed by later fast completions. The client saves every clean/churn sample in CSV after the packet loop.

A second thread uses a separate owned VA space pinned live by an idle owned channel, with reversed physical offsets for4174 small4KiB maps at a2MiB stride, deferred invalidates, unmap/invalidate, and sparse-reservation recreation, for at most32 rounds and20000 churn packets. It never churns the packet's operand/ring mappings. Setup/refusal, absent overlapping packets, or no complete round is INVALID. No claimed large-apply coverage is inferred from the requested map count: the host must actually record a qualifying apply in the named churn space.

CODE READ, existing `forced_user_ring` and `birth`: ordinary Linux passthrough bypasses the Windows USER walk gate. The controller must explicitly use the existing `KF3_DEBUG=1` plus `KF3_DEBUG_FORCE_MIGRATABLE=1` observation profile and confirm the exact guest client/channel's USER birth. This task adds no routing exception. A normal Linux route is a separately labelled control, not evidence against the USER-gate hypothesis.

## Scoring without a false PASS

Run `scripts/bench/ens4-linux-gate.sh` only inside an authorized guest; it collects the client log and complete CSV, not a final mechanism verdict. The offline scorer uses the exact owned client/channel, its host token, the named churn VA space, a qualifying large apply, a terminated gate dump and a hold of that channel. It recomputes p99.9/bound from the complete CSV and checks the client's result. Other clients' same-numbered channels, fleet IRQs, unrelated/small applies, truncated dumps, dropped samples and missing calibration receipts cannot certify PASS.

```sh
python3 scripts/bench/ens4_score.py \
  --client-log <arm>/client.log --qemu-log <arm>/qemu.log \
  --csv <arm>/latency.csv
```

The scorer emits PASS/FAIL/INVALID and exits0/1/2. Its `worst_completion_ns` is the packet-specific Linux client endpoint. The scorer requires host gate exposure at boot level; it is not a matched Windows ETW packet-causality claim. Keep raw sample files and full logs with the controller's source/executable/flag seal. No host runner, lock or VM was invoked by this lane.

## Per-boot completion-observation metric

CODE READ, [NVIDIA's public SET_REF method description](https://raw.githubusercontent.com/NVIDIA/open-gpu-doc/master/manuals/turing/tu104/dev_pbdma.ref.txt), lines3597–3613: method0x50 implicitly drains prior engine work and completes a system-memory barrier before updating reference. An explicit WFI instruction is not required for that marker. The class header maps SET_REFERENCE to0x50. Raw GET, IRQ or an unassociated reference word still cannot close a sample.

CODE READ, `gatelog/ens4.rs` and `userpump.rs`: `ens4_worst_gate_apply_complete_ns` is the largest observed gated-publication interval through a covering SET_REF completion observation. For an entry already announced when gated, the start is its observed announcement. If the gate preceded any PUT observation, the start is the first observed gate hold and `ens4_metric=gate_start_proxy` marks the unknown actual arrival. This proxy can over- or underestimate true guest submission latency; it is not silently labelled exact. Multiple entries share a marker-covered batch, using the oldest relevant start. Apply already lies within the gate interval and is not added again.

The observer records all three actual marker publication paths only after successful private PUT. Direct pointer/gate/batch PUTs also update the existing ENS3 progress fields after success. At successful publication it records the marker and binds preceding guest-entry hooks; at turn end it consumes only `Marks::seen` from an ordinary observation actually used by the relay. Old/future marker counts, stale samples, inferred cursor resyncs and completed-before-hold work cannot fabricate a completion for new work. At most512 marker associations per tracked channel are retained; overflow and retirement gaps are explicit. Completed maxima survive native token reuse and diagnostic eviction. Nothing decides scheduling, retirement or authorization from this data.

Status and first153 dumps print the value, source kind, completed/pending batch counts, association loss and untracked channels. No matched completion yields NA; an open hold or marker wait also has an age lower bound in the status/dump. Facts OFF prints NA/off and performs no new observation. These are observer timestamps, not hardware timestamps or proof that Windows recognized completion. Marker-free/open-packet paths remain pending/unavailable. A hostile guest can falsify its own tagged-reference diagnostic, so it is never a security predicate. The known-arrival subset and matched Windows ETW remain needed for packet-specific causal attribution.

The prospective dose series, early-stop/bisection rules, Linux baseline factor and falsifiers are in [PREREG.md](../../traces/ens4_20261018/PREREG.md). The existing sparse default, mirror rules and v3 refresh contract remain unchanged.

## Deterministic relay and external completion replay

CODE READ, `kf-chan/tests/ens4_support/mod.rs`: the extension reuses the existing adversarial lifecycle World, simulated I/O and production relay micro-steps. An apply adversary begins/ends a hold at any step. The exhaustive scenarios cover Scan/Pointer, Window/migration and full private-ring credit; seeded scenarios cover migration and CreditWait with exact replay. Apply completion announcing work is an explicit environment assumption; the unchanged fairness and handoff suites cover registration separately. No loom/shuttle dependency is introduced because this is a protocol/environment state space rather than a small atomic primitive.

CODE READ, `ens4_replay.py`: an additional seeded scheduler controls external publish/host-complete/edge/guest-observe ordering. A synthetic dropped edge validates the delivery detector; seed42 shrinks to three events. This is not a discovered current-code race. The converter accepts timestamped ENS3 apply, hold, PUT/publication and actual marker records, retaining cumulative NSI/progress as coverage notes. It emits incomplete coverage until packet-specific guest/ETW sidecar evidence and a complete observation horizon are joined. No real failing boot trace was collected here; its failing-first regression remains pending authorized capture. RM/GSP timing and guest-driver recognition are outside these models. A green result means “no reproduction”.

CODE READ, `ens4_packet.rs`: first153 output distinguishes unpublished work, covered host completion still unobserved, and host-completed work with guest delivery unknown. The latter is only a class(ii) candidate. It names the latest observed cohort and prints affected_packet=unknown; it does not claim to identify the guest's timed-out packet. Marker publication and validated observation enter the existing bounded history. Cheap default-on ENS4_UNISSUED_SEEN and ENS4_SEQUENCE_REGRESSED counters check successful publication and monotonic progress without extra GPU reads; they are passive diagnostics.

The controller replaced remaining ENS4 scope with ENS5 before hardware authorization. The Opus summary subsequently supplied a concrete saturated sparse-admission candidate; ENS5 investigates that separately. ENS4 leaves production gate policy unchanged and makes no root-cause claim from its synthetic delays.
