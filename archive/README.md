# archive

Frozen reference material. Nothing here is built, tested, maintained, or
intended to run.

## `nvkvm/`

The original **nvkvm** C research prototype, snapshotted at commit `bac00b6`
and imported as a single squashed commit (its own history was not carried
over).

It is kept because kayfabe's design references its **Mode-2** work directly —
the differential oracle, the address table, the forwarding model, and lessons
#11–#14. When a kayfabe design doc says "the C artifact does X", this is the
code it means.

Two things it is not:

- **Not a supported project.** It does not build here, is not covered by CI,
  and will not be fixed.
- **Not the maintained descendant.** That is
  [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv), which was forked from
  this prototype and deliberately **excludes** Mode 2 — Mode 2 is a research
  artifact, and nvkvm-pv ships only the paths that are tested.
