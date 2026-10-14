# V3 provenance: where kayfabe's data and interfaces come from

**STATUS: LIVE, 2026-10-11 (DRAFT: wording to be reviewed by a lawyer before the public release).**
Owner rulings: `docs/OWNER_RULINGS.md` §AF (per-die constants) and §AG (reverse engineering stays private).

This page states the rules kayfabe follows. It deliberately says nothing about what any analysis found.

## 1. Sources

Every per-die constant, register offset, class set, control layout or reply shape in kayfabe comes from one of these, in this order:

1. **Computed** from kayfabe's own state or from class tables.
2. **Open sources**, pinned and licence-checked: NVIDIA's open GPU kernel modules (`ogkm`) and open documentation; other open projects that already
   carry the information (nouveau, nova, envytools, gVisor nvproxy, and similar).
3. **The host's own driver**, queried at runtime with unprivileged controls.
4. **Measurements** on real GPUs (our own and a rented fleet of many GPU types), published as a table. The published table is the truth.
   Measured entries never contain anything board- or user-specific (UUIDs, serials, VBIOS/PROM contents, MAC addresses).

Each table entry carries a source tag (`ogkm`, `nvidia-doc`, `nouveau`, `nova`, `measured:<die>,<driver>,<date>`, and so on).
**CI refuses an entry tagged `blob`.** The build and every derivation script fetch the open sources and the published table; none requires or
contains a decompilation step.

## 2. Reverse engineering

- Examination of closed binaries is limited to **diagnosis and verification**: finding out why something fails, and confirming that a table or a
  behaviour is right. It is **not** a data source. No interface, layout or constant is taken from it.
- It is kept small, and only where the other sources do not answer the question (interoperability need, minimum scope).
- Its working material is **not published** and stays outside this repository, in access-controlled private storage.
- A finding that matters is restated as a **behaviour-level verification item** ("the component requires call X to succeed") and is then
  confirmed with a test or a measurement against the sources in §1 before any code depends on it.
- Public documents, commit messages and traces do not carry offsets into, symbol names from, or annotations of closed binaries.

## 3. What this is not

This is not a clean-room process: the people and agents who built kayfabe are the same ones who may have run a diagnosis. The protection is
the **source rule** (§1: data comes only from listed sources, and the tag proves which), the **private-material rule** (§2), and review.

## 4. Release checks

- Before a public release the whole git history (not only the tree) is searched for analysis material and closed-binary offsets.
- The control table is checked by CI for source tags (no `blob`, every entry tagged).
- This page is reviewed by a lawyer before it is published.
