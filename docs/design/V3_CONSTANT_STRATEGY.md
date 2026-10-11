# V3 per-die and per-driver constants: the strategy

**STATUS: LIVE, 2026-10-11 (strategy; nothing here is implemented yet).** Owner rulings: `docs/OWNER_RULINGS.md` §AF (sources, source tags,
published table), §AG (reverse engineering private), §AH (this strategy). Provenance rules: `V3_PROVENANCE.md`. The inventory this strategy works
through: `V3_CONSTANT_INVENTORY_20261011.md` (being written; until it lands this page names no item as open).

## 1. Goal

kayfabe must work on a wide range of dies and guest driver branches, not only the ones we traced. Every value a guest can see that is not
derived or computed must come from a published, source-tagged table, and an unmeasured die or branch gets a defined fallback.

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
4. **Reverse engineering** for the residue, per §AG and `V3_PROVENANCE.md`, with a legal review first.

## 5. Unverified assumptions (each has a falsifier above)

- A kernel module can issue an arbitrary control id straight to the firmware (inferred from the open driver's structure, not checked here).
- The firmware does not itself reject controls the open tables lack.
- A display-common object can be allocated on a GPU with no monitor attached (rented machines have none). Answers measured without a monitor are
  die constants only; monitor-dependent values stay authored by kayfabe.
- A Windows rental is needed only for behaviour tied to the Windows driver; this may be empty if the raw-GSP spike works.

## 6. Priority

Release-coverage work, not the current blocker. The OpenGL failure on the RTX 4070 is not a constants problem (the replies already equal the real
GPU's), so the sweep runs beside the P0 work and does not replace it.
