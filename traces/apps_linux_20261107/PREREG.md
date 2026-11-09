# The Linux app matrix at the current tree: pre-registration, 2026-11-07

STATUS: DESIGN-ONLY, 2026-11-07 (lane appslinux). Written and pushed BEFORE any box was rented. Results go
to `README.md` in this directory, never into this file (a correction is added above the text it corrects).

## Why

The owner's ruling (2026-11-07): the release gate must include the old ~70 CUDA/graphics/AI apps in the Linux
guest, in full, and the Worker payload has no apps stage today. The newest recorded app result is `60/65 apps`
at kf3 `4c48ca0c` and `61/65 apps + 6/6 probes` at `2830988f` (`docs/design/V3_APP_MATRIX.md` R3), both on an
RTX 3060 (GA106); R4 (`738c90e5`, RTX 3070) was partial. No Ada card has run the matrix, and no run exists at
the current tree. A gate list built from numbers that old would be a guess.

## What runs

- Tree: `origin/next` at `a9dee7b2c` (contains `e44e38b82`), branch `claude/appslinux-20261107` adds only this
  directory, an `apps` stage draft and its tests. `git diff --stat origin/next -- crates qemu cuda` is empty and
  is quoted in the README.
- Box: ONE direct Vast rental, label `kf-appslinux-20261107`, RTX 4090 (AD102) preferred, RTX 3090 as the
  fallback; `vms_enabled=true`, the KVM image, disk request >= 200 GB, refuse < 120 GB free. Excluded hosts
  392559, 224600, 96607 and machines 103782, 36135. Lease <= 3 h, one re-rent allowed.
- Provisioning is the payload's own path as far as the box allows: `provision_full.sh` (host driver, tree,
  raw client, fast guest, kf3 build), then `scripts/apps/build_bundle.sh` (CUDA 12.6 toolkit, cuda-samples
  v12.5, gpu-burn, llama.cpp `4b1a27f`, clpeak 1.1.2, vkpeak 20250531), `setup_side.sh` on the host,
  `provision_guest_apps.sh` for the guest image. Every download is official or public; none needs a secret and
  none is put on the box. Sizes and times are recorded per step.
- Matrix: `bash scripts/apps/run_apps.sh x list` = 71 rows. `geekbench_gpu` is NOT run: `docs/DOWNLOADS.md`
  marks Geekbench "not allowed (scripts need Pro), manual", and the free mode uploads the result to a third
  party. It is recorded `NOT_RUN policy`, not as a pass or a fail. The runnable set is 70.
- Order (bare metal first, CLAUDE.md): `apps_matrix.sh host <run> all`, then `apps_matrix.sh guest <run> all`
  with `APPS_PER_BOOT=100` (one boot, wedge probe on, as R3's `m20`), then every non-PASS app alone in a fresh
  boot (the harness's phase 2). Start marker and exit line in each log; `apps_matrix.sh` is polled by pid and
  by its `HOST_DONE`/`GUEST_DONE` line, never by "the file stopped growing".
- Recorded per app: host verdict, guest batched verdict, guest isolated verdict, `secs`, the key output line
  (a figure where the app prints one), guest/host Xid counts, kf3 refusals.

## Hypotheses and falsifiers

Each is stated before the run; the result is reported either way.

| id | hypothesis | falsifier (the result that proves it wrong) |
|---|---|---|
| H1 | Bare metal on this box passes >= 65 of the 70 runnable rows. The failures, if any, are app/harness/hardware limits, not kayfabe's. | Fewer than 65 host PASS. Then the matrix predicates or the bundle (built `sm_86`, run on `sm_89`) are not a usable baseline on Ada, and the gate list is built from what passes on the host only. |
| H2 | The guest passes the same set as the host within +-3 apps (batched, then isolated re-runs), i.e. >= 62 of the host's PASS rows. | More than 3 host-PASS apps fail in the guest ALONE in a fresh boot. Those apps are kayfabe defects; they are listed, never dropped from the table. |
| H3 | The four UVM demand-paging rows (`UnifiedMemoryStreams`, `UnifiedMemoryPerf`, `conjugateGradientUM`, `attach_verify`) still fail in the guest (STATUS_DETAIL: the UVM guest plane is not built). | Any of the four passes alone in the guest. Then the gate list takes it. |
| H4 | One guest boot holds all 70 rows without a process-budget wedge (R3 `m20seq`: 100/100). | An `APPS_WEDGE` line or a GUEST_DEAD row appears in the batched run on a row whose isolated re-run passes: the row is classed `ORDER_SENSITIVE`. |
| H5 | The owner's middle-tier definition "keeps ~70 apps in ~10 min" does NOT hold: the host pass alone takes more than 10 min (gpu_burn 60 s, llama_bench, vkpeak, clpeak, Blender CUDA+OptiX, hashcat) and the guest batched pass more than 20 min. Expected ranges, written now: host 12-35 min, guest batched 25-70 min (nested KVM costs a VM exit 10-40x bare metal). | Host + guest batched wall time together under 20 min on this box. Then ~10 min per side is real and the stage fits the middle tier as is. |
| H6 | Provisioning the apps costs 45-120 min on a fresh box (downloads ~20 GB across host and guest, toolkit, llama.cpp and samples builds). | Under 30 min. Then the box could build the bundle inside the payload; over 120 min means it must arrive prebuilt. |

## Classification of a failing row (decided now, applied mechanically)

1. Host FAIL, TIMEOUT or NOTRUN on the same box: `HOST_LIMIT` (app, harness or hardware). Not kayfabe's
   (CLAUDE.md, "bare metal first"); a documented exception. A NOTRUN is a provisioning gap: fixed once, then
   re-run; if still absent it stays `NOTRUN` with the reason, and is not counted as a pass anywhere.
2. Host PASS, guest PASS (batched or isolated): `GATE` (required).
3. Host PASS, guest batched FAIL, isolated PASS: `ORDER_SENSITIVE`. Listed as a kayfabe finding (a boot that
   degrades is a defect), and runnable in the gate in its own boot.
4. Host PASS, guest FAIL alone in a fresh boot: `KAYFABE_DEFECT`, with the first host/guest Xid line, the first
   kf3 RC/refusal line and the app's own error. Never hidden, never "flaky" without a second isolated run.
5. A failure with a provider fault on the box (the box lost, GPU fallen off the bus, host Xid before the guest
   started, the same row fails on the host in a re-run): `UNATTRIBUTED`, re-run once on the same box; not
   counted either way. A same-box Linux-host reference is the only thing that can attribute a rental failure.

PASS predicates are the harness's own (`run_apps.sh`: rc 0 AND the app's success string AND no `CHECK .* FAIL`).
They are not edited to change a verdict. A predicate bug found on the host is reported and fixed in a separate
commit with its reason; the table then says which predicate version each number used.

## Budget and stop rules

8 USD in all, 6 h wall. Expected: 3-4 USD (4090 at about 0.67 USD/h with a 200 GB disk, 4-5 h). Stop and report
at 7 USD. Teardown by exact id, with `vastai show instances-v1 --raw` no longer listing it; the box output is
data. No `instance_api_key`, token or presigned URL is ever printed or written.
