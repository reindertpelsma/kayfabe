# V3 per-die and per-driver constants: the strategy

**STATUS: LIVE, 2026-10-11 (strategy; nothing here is implemented yet).** Owner rulings: `docs/OWNER_RULINGS.md` §AF (sources, source tags,
published table), §AG (reverse engineering private), §AH (this strategy). Provenance rules: `V3_PROVENANCE.md`. The inventory this strategy works
through: `V3_CONSTANT_INVENTORY_20261011.md` (being written; until it lands this page names no item as open).

## 1. Goal

kayfabe must work on a wide range of dies and guest driver branches, not only the ones we traced. Every value a guest can see that is not
derived or computed must come from a published, source-tagged table, and an unmeasured die or branch gets a defined fallback.

## 1a. The compatibility axes at a glance

| axis | what varies | how it is tackled | open risk |
|---|---|---|---|
| **Die / display IP version** | capability bits, class sets, per-die constants (e.g. GMMU static info differs on AD102 vs GA102/GA106) | tiers §2: derive first; else measure per die (Vast sweep, home display machines RTX 4070 / 3050 / 3050 laptop, contributor tool); no "same family" assumption (§3); unmeasured die = clean fallback | dies nobody can measure with a head (§4 step 3b, §9) |
| **Guest driver branch / firmware** | control ids are renumbered (ogkm 555.42.02), layouts change inside a branch (`0x20802a0f` 24/28/32 B), controls appear only from some version | request census per driver version under kayfabe; ogkm across all tags; per-(die, branch) table keys; a supported driver set (§7) | monthly Windows drivers add controls |
| **Host driver version** | what the unpatched host driver answers unprivileged; ABI of host controls | tier 2 (delegate to the host where safe), `kf_abi::hostabi` rows, driver matrix, patched host as an optional tier (`V3_HOST_PATCH_LIST.md`) | host RM core ships as a binary object; patching needs a source build |
| **Monitor / sink state** | connector table, EDID, modes, HDCP, DP vs TMDS | kayfabe authors it from its own virtual monitor; real behaviour is an oracle test on display machines; forced-connector measurement on rentals (§4 step 3b) | DP/MST/HDMI sinks, multi-head |
| **Host environment** | nested KVM exit cost (10-40x), IOMMU, DMA address limits | per-box ratios; the H5 constraint is host-specific, not a die constant | Windows under nested kayfabe is untested (§8) |
| **Guest OS** | Windows-only controls, Windows-only behaviour | Windows tracer / replay (§4), acceptance layer (§8) | headless Windows may not issue display controls (§5) |

Acceptance for every axis: the nested-run test layer with falsifying tests, mutation checks and a native differential (§8).

## 2. The tiers (owner, 2026-10-11)

Every open item is checked in this order. It moves to the next tier only with a written reason why the tier above does not work.

1. **Derive or compute:** ogkm (the whole tree: generated code and the Windows-specific code it carries, not only what Linux builds; and **any ogkm
   version, older tags and newer ones, not only 595.84**: an item missing from one version may be defined in another), nouveau/nova,
   other open projects that already carry the table, kayfabe's own state, rules checked against class tables.
2. **Unprivileged host ioctl at startup:** kayfabe starts unprivileged. Only what an unpatched host driver answers without extra permissions.
3. **Privileged measurement, run once per die and driver in a sweep** (a script as root, a kernel module, hardware access, on rented machines).
   This is measurement, not reverse engineering, provided the tool only reads or probes the interface and never patches or decompiles the closed
   side. Entries are tagged `measured:<die>,<fw/driver>,<date>`.
4. **Reverse engineering**, only for what remains, private and verification-only unless the owner amends §AF (see `V3_PROVENANCE.md`).

## 3. Rules for the table

- **Measured per die and per firmware/driver branch by default.** "Same family" is never assumed. A family is a compression, not an input.
- **A set of rows may be compressed into one family rule only when** every die measured in that family agrees AND a mechanism explains why (an
  ogkm derivation, a class table, a fuse-derived rule). Agreement without a mechanism is recorded as "consistent so far", not as a rule.
- **A rule is a test oracle**: every new measurement is checked against it, so a die that breaks it is found by the sweep.
- **Unmeasured die or branch: the clean fallback** (§AE): refuse by name, count it, claim nothing; never use a neighbour's row. (Precedent:
  `kf-disp` `dispcaps.rs` claims none of the three hand-kept bits for an unobserved display IP version.)
- **Inferred rows are labelled inferred.** The display feature bits are observed on one die (DISPv0404 on AD104, one guest driver). The statement
  that AD102..AD107 share them rests on a shared version id with no mechanism, so it is `inferred, unverified`; the sweep measures those dies.
- Board- and user-specific data (UUIDs, serials, VBIOS/PROM contents, MACs) is never collected.
- CI refuses an untagged entry and any entry tagged `blob`.

## 4. Plan

0. **Inventory** (running): every guest-visible value not derived, with its evidence and the axes it varies on. The tier checks and the sweep's
   probe list come from it.
1. **Spike: tier 3 on the home RTX 4070 (about one day).** A small kernel module built from the open driver sends a raw GSP RmControl for a given
   control id and parameter block, below the open driver's control table. Compare its replies with the VFIO captures. Falsifier: the module cannot
   issue a control the open tables lack, or the firmware refuses it. Then those items go to tier 4, and the Windows-side tracer (a passive reader of
   the driver's GSP queues, validated against the VFIO capture on the same boot) is the fallback.
2. **Variance study:** 4 to 6 dies across generations, plus 2 to 3 driver branches on one die. It tells us which items vary by die, display IP
   version or firmware branch, and how big the table is. It does not license inference about unmeasured dies.
3. **Sweep and tool:** an automated per-die run on rented machines (one boot per probe batch, a time limit, reboot on a wedge), ordered by how often
   a die is rented. The same measurement as a root-run tool (`kayfabe-measure`) that writes a table entry with board data stripped, so users can
   contribute dies beyond the rentals.
3b. **Forced-connector experiment (display behaviour on headless rentals):** on a rented Linux VM with root, make the driver treat a connector as connected with a custom EDID
   (DRM `video=...:e` or the driver's connected-monitor option; an HDMI/DVI output needs no sink handshake), then modeset and flip, read the frame CRC capture through NVKMS,
   and trace the display engine's behaviour (latch timing, vblank cadence, colour-program acceptance). Falsifier: one modeset on a forced connector on one rental. It is
   measurement, not reverse engineering. Datacenter dies with no display engine need nothing here (headless is their real state); workstation and consumer dies of one
   display IP generation share the IP version key but are still measured per die by default (§3).
4. **Reverse engineering** for the residue, per §AG and `V3_PROVENANCE.md`, with a legal review first: only display-engine behaviour of display IP versions that nobody can measure
   (no owned machine with a head, forced-connector failed), done as a two-party spec process (one session writes a spec of facts, another implements from the spec only), tagged
   `re-spec:<id>`, and verified by the acceptance layer (§8).

## 5. Unverified assumptions (each has a falsifier above)

- A kernel module can issue an arbitrary control id straight to the firmware (inferred from the open driver's structure, not checked here).
- The firmware does not itself reject controls the open tables lack.
- A display-common object can be allocated on a GPU with no monitor attached (rented machines have none). Answers measured without a monitor are
  die constants only; monitor-dependent values stay authored by kayfabe.
- A Windows rental is needed only for behaviour tied to the Windows driver; this may be empty if the raw-GSP spike works.

## 6. Priority

Release-coverage work, not the current blocker. The OpenGL failure on the RTX 4070 is not a constants problem (the replies already equal the real
GPU's), so the sweep runs beside the P0 work and does not replace it.

## 7. Additions agreed 2026-10-11 (owner: "I agree with your corrections")

- **Demand filter before the tiers.** Only rows that change guest behaviour go through the tiers. Run 641 booted, signed in, ran Edge and the CUDA probes
  with 61 controls refused, so most inventory rows may not matter. Test each refused group (answer it, watch for a behaviour change), and tie every
  app-matrix failure to the control it hit. Rows that matter get the per-die sweep; the rest stay a counted clean fallback. This is the largest cut to
  the reverse-engineering list and does not lose hardware variation for the rows that matter.
- **Delegate to the host driver wherever it is safe** (tier 2): kayfabe authors the same read-only request from validated fields (never guest bytes) and returns
  the real reply, so no per-die table is needed. Tables are for what the open host driver lacks.
- **Supported driver set.** Windows drivers change monthly; define the supported set (the newest N branches plus one long-lived one), clean fallback for the rest.
- **Opt-in report of refused controls** from `kayfabe-measure`/kayfabe (anonymised counts per die and driver branch) to learn which dies and drivers users hit,
  so sweeps are ordered by real demand. Nothing is collected without the user's opt-in.

## 8. The acceptance layer: a nested kayfabe run on each rented die (owner, 2026-10-11)

kayfabe emulates the display engine, so a Windows or Linux guest under kayfabe on a headless rental still sees a head. A nested run per die is therefore the
acceptance test for every value that did not come from a same-die measurement (an unmeasured die's clean fallback, a value from another die, a reverse-engineered
value). Owner: the test must be rigorous. What rigorous means here:

- **Every row has a falsifying test.** A row that came from reverse engineering or a fallback gets a behaviour-level verification item (`V3_PROVENANCE.md` §2) naming
  the guest feature that depends on it (HDR, overlay, multi-monitor, colour program, flip timing, ...). A row with no such test is not "verified by a pass".
- **Mutation check: prove the test can fail.** For each row, change the answer to a wrong value and confirm the suite fails. A test that still passes with a wrong value
  is not evidence about that row and is listed as uncovered.
- **Differential against the real driver on the same die.** The same app matrix runs natively on a Windows rental (baseline: 99/100 on the vast RTX 4060 Ti) and under kayfabe
  on that die. Compare render checksums, engine usage, event-log errors and per-app verdicts; every difference is a finding. A pass on both is not enough if the outputs differ.
- **Cover the features, not only boot:** sign-in, Edge video (overlay), OpenGL/D3D/Vulkan, NVENC/NVDEC, CUDA ladder, HDR and multi-monitor where the die supports them,
  hot-plug/resize, and a soak (no TDR for 60 minutes).
- **A pass is evidence only for the exercised paths.** Features with no test stay on the list as uncovered, not as passing.
- **Unverified:** Windows has not been run under kayfabe on a nested Vast host (VM exits cost 10-40x; watchdogs and the TDR timeout may fire for reasons unrelated to
  constants). First check: one Windows boot under kayfabe on a rented Linux VM; tune budgets before reading a failure as a missing answer.

## 9. Seventh class: display-state rows for dies without a head on hand (owner discussion 2026-10-11)

Rows whose reply depends on a lit head or a monitor, on a die we can reach only by rental. Route in this order: authored by kayfabe from its own virtual monitor (the rules and formats
from any die of the same display IP version that has a head) -> forced-connector measurement (§4 step 3b) -> contributor measurement on a machine with a monitor -> clean fallback
for the die -> reverse engineering only for an unreachable display IP version (§4 step 4). The owner's machines with display: RTX 4070, RTX 3050 (desktop; GA106 8 GB or GA107 6 GB,
to be confirmed), RTX 3050 laptop (whether its dGPU drives a head is to be confirmed; a MUXless laptop is a headless reference); the owner may have a Turing card and can get one Blackwell.

## 10. Coverage must not wait on others, and iteration needs access (owner, 2026-10-11)

- **Community contributions are the long tail, never the critical path** (slow, and a barrier to adoption). Launch coverage is proactive: a market-chosen set of dies (about the top 20 across
  RTX 30/40/50 consumer and workstation RTX A/Ada) through the Vast sweep, the owner's display machines, forced-connector measurement and the nested acceptance run (§8); reverse
  engineering (§4 step 4) for the residue of that set; every other die gets the clean fallback.
- **The fallback must be a tested, working mode**, not only a defined one: run the acceptance layer with the fallback answers (HEAD already claims none of the optional display feature bits,
  commit `b88f2f60`); if it passes, an unmeasured die works with fewer optional features and contributions only add fidelity. This is the number to get before stating launch criteria.
- **Iterate on access, not logs** (owner: "I can't iterate on failures, so ssh to a host is infinitely more valuable than three log files"). A failure we can reproduce over ssh on a rental is fixed in a loop;
  a log file from a user is not. So rentals are the primary source, and where a die can only be reached through a volunteer, the measurement tool emits a **replayable capture** (requests, replies and the state they
  depended on, board data stripped) that becomes a GPU-free regression test, so iteration continues offline. A time-limited ssh session to a volunteer machine is better still, if a volunteer offers one.
- **Measure at install** when a measurement needs only a root RM client (no kernel module): once per user machine, cached locally. Depends on the raw-GSP spike (§4 step 1).
- **Vast reach, measured 2026-10-11 (one snapshot; offers change):** 29 rentable VM-capable offers (`vms_enabled=true`), 11 distinct GPU models: RTX 4090 (7), RTX 5090 (4), RTX PRO 5000 (3), RTX 3070 (3),
  RTX 3060 (3), RTX 6000D (2), RTX 3080 (2), RTX A4000 (2), RTX 5070 Ti (1), RTX 3090 Ti (1), Tesla P100 (1). That is Ampere, Ada, Blackwell consumer and workstation and one Pascal datacenter card, and no Turing at this moment.
  Re-query before each sweep; the sweep tool should record which models were reachable and when.
