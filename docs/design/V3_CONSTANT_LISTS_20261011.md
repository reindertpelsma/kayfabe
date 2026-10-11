# V3 constants: the three lists (obtainable / measure / reverse engineering)

**STATUS: LIVE, 2026-10-11 (first merge of the tier checks; not demand-filtered; classification is mechanical, review the rows).**
Inputs: `v3_constant_inventory.tsv` (140 rows, one die, one Windows driver 580.88, one monitor), `tier_checks/tier1_ogkm.tsv` (all 221 ogkm tags,
515.43.04..615.78.08), `tier_checks/tier1_thirdparty.tsv` (nouveau, nova, envytools, open-gpu-doc, Mesa, gVisor nvproxy, tinygrad mirrors, nvkvm-pv),
`tier_checks/tier2_compute_host.tsv` (computability, unprivileged host ioctl, privileged measurement, Vast). Output: `tier_checks/merged_classification.tsv`.
Strategy and tier order: `V3_CONSTANT_STRATEGY.md`, owner rulings §AF-§AH. MEASURED = found in a source file or capture; INFERRED = an agent's reading.

## Rule used to classify (mechanical)

- **ALLOWED-FAMILY** (11): hand-maintained family rows, policy.
- **1-OBTAINABLE** (74): the layout is in open source (or the reply is trivial: status or scalar) AND the value is derived (ogkm rule), computed from
  kayfabe state, or answered by an unprivileged host ioctl.
- **2-MEASURE-LAYOUT** (17): the value looks derivable or host-served, but the layout/id is in no open source, so it is known only from one observation.
- **3-MEASURE-SWEEP** (28): not derivable, not host-served; a privileged measurement is feasible on a headless rented machine and the reply is not display-state dependent.
- **4-MEASURE-NEEDS-DISPLAY** (5): feasible only with a lit head/monitor, or authored by kayfabe from its own state.
- **5-RE-CANDIDATE** (5): fails every tier above. (The compute agent's own strict list was empty; mine is stricter on timing/behaviour rows.)

## The lists

**2-MEASURE-LAYOUT:** ctl-00809908, 20808524, 2080853b, 20809004, 2080a02f, 2080a088, 2080a619, 2080a630, 2080a70a, 2080a801, 2080b201, 20801111, 20809001,
hostask-2080a028, hostask-2080a026, const-get-display-mode-73011a, const-gssreplay-rows. (Mostly opaque blocks. The compute check says the bit-15 legacy
controls are unprivileged and forwarded to the host's GSP, but that is INFERRED from ogkm 595.84: nothing was measured at the host's 595.91.07, and kayfabe's own gate
causes the `HOST-ABI REFUSED` lines. **One probe at the host settles it** and may move most of these to 1-OBTAINABLE as delegation.)

**3-MEASURE-SWEEP:** ctl-00730122, 00730128, 007302a2, 007302a5, 20800a2e, 20800a3f, 20800a80, 20800a87, 20800ab8, 20800b03, 20802801, 20802806, 20802a12, 0073014b,
007302a3, 00731369, 2080121f, 50700709, 50700118; const-echo-730122, reply-730128, display-model-authored, display-caps-page-probe, head-timing-en-base,
ce-fault-method-buffer, gmmu-static, user-register-access-map, gfxpool-sizes. (Only 4 of the compute check's 14 sweep candidates need no kernel module.)

**4-MEASURE-NEEDS-DISPLAY:** ctl-00730293, 007302a6, const-hdmi-path-probe, const-evt-stat-loadv, const-blank-state (sink/monitor-dependent: kayfabe authors them from its
own state; the real-GPU behaviour is an oracle from the display reference machine, not a constant).

**5-RE-CANDIDATE:** ctl-20801222 (never issued in any real capture), const-lut-mirror-offset256 (decoder acceptance is hardware behaviour), const-core-latch-vblank,
const-head-timing-frame-edge (timing behaviour), const-sw-runlist-object. These are timing/behaviour or never-observed rows: first try the display reference
machine (VFIO observer with a monitor) and the raw-GSP spike; reverse engineering is the last resort.

## Findings that change kayfabe (MEASURED unless marked; verify, then open a gap row each)

- **Captured value served for the wrong die.** `GMMU_GET_STATIC_INFO`: the AD104 returns 0x54000 / 0x220c20; kayfabe serves 0x31000 / 0x120c20 (the GA102/GA106 values in the
  old traces). The old traces show AD102 differs from GA102/GA106 too.
- **5070 caps:** the die returns 0x24, kayfabe serves an empty table.
- `0x20809064`: the reply depends on the request; the single cudartinit capture is wrong for the Windows request.
- `0x730122` is not `GET_CONNECT_STATE` of the guest's era: nouveau/ogkm name it that (16 B) only in 515..550 and from 565 that control is `0x730108`; the observed 8-byte echo matches neither.
- **Control ids are renumbered at ogkm 555.42.02** (GET_CAPS_V2 is 0x730138 before, 0x730101 after): a numeric hit in an older tag can be a different control.
- Layouts change inside one branch: `0x20802a0f` 24 B to 28 B at 580.94.02 (570/580/610: 24/28/32). `0x730280`/`0x730282` exist only from 590.44.01.
- `0x208081ea` IS defined in ogkm 575.51.02..580.178.04 with a value rule (`gpu_user_shared_data.c:439-459`); the inventory said undefined.
- `NV2080_CTRL_GR_GFX_POOL_MAX_SLOTS` is 64 in ogkm; kayfabe uses an invented 4096.
- Report-semaphore `OFFSET_UPPER` is 7:0 up to Ada but 24:0 on Hopper/Blackwell (Mesa headers) against the repo's "identical on Hopper" comment (INFERRED until checked).
- No tag has the physical-RM source: 60 of 67 ids with a generated method row are routed to the GSP firmware, so ogkm gives layouts, never reply values.
- Host driver 595.91.07 is the open module, but its RM core ships as `nv-kernel.o_binary` in the DKMS tree: patching control tables or sending raw GSP RPCs needs a build from the GitHub source.

## Not yet done

- The demand filter (`V3_CONSTANT_STRATEGY.md` §7): which rows change guest behaviour at all (run 641 works with 61 refused controls).
- The host probe for the 2-MEASURE-LAYOUT rows, and the raw-GSP spike (strategy §4.1). Both need the GPU lock.
- Size-only layout comparisons: field offsets were not compared against payloads.
- No headless capture exists; every "not display-state dependent" holds only across five captures that had a monitor.
