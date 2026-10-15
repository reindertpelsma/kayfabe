# Where this directory comes from

**STATUS: LIVE, 2026-10-15.** `broker/` is the host display-broker program (window, compositor connection, input grab,
cursor), the other half of `crates/kf-broker`. It was imported from the owner's repository
[nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv) (Apache-2.0 / GPL-2.0, the same licences as here) at commit
`badf2d707da4` (branch `broker-cursor-gpucopy`, `src/broker/` plus the two headers it includes from `src/common/`:
`nvkvm_broker_proto.h` and `nvkvm_uidmap.h`, now in `broker/common/`). That is the revision installed on the trusted host
(`/opt/nvkvm-broker/REV`) and used by every Windows and Linux display run so far.

- **From now on this copy is the one to change.** Changes to the wire protocol go into `broker/common/nvkvm_broker_proto.h`
  and into `crates/kf-broker/src/wire.rs` together. nvkvm-pv keeps its own copy and is not updated from here automatically.
- Build: `cd broker && make` (prints which backends were compiled in), `make check` runs the unit tests and `selftest.sh`.
  Debian/Ubuntu build dependencies are listed at the top of the `Makefile`.
- Only paths changed in the import: the two headers moved to `broker/common/` and the include paths and `-I` flag follow.
  Generated Wayland protocol files are not committed (the Makefile generates them).
- `crates/kf-broker/proto/nvkvm_broker_proto.h` is still the older vendored copy (nvkvm-pv `368d2db`); reconciling it with
  `broker/common/nvkvm_broker_proto.h` (the `proto_mirror` test) is open.
