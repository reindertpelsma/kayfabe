# Native read-only timer prerequisite

2026-10-05, borrowed PC `172.22.1.20`, RTX 4070 AD104, NVIDIA open driver 595.91.07.
Product/probe revision **831f6bd755be7abd2e6b219fd377a692e5f78126**.

`native-v3-unprivileged.txt` records three successful native `NV01_TIMER` alloc/map/read/drop
cycles. Executed with uid/gid 65534, empty groups, all capability sets zero, and no-new-privileges:

```sh
setpriv --reuid=65534 --regid=65534 --clear-groups --no-new-privs \
  --inh-caps=-all --ambient-caps=-all --bounding-set=-all \
  /var/lib/kf-windows-20261005/target/release/kf-timer-probe
```

The host queried BAR0 base `0x9000`; the compiled SDK map is 1044 bytes (low/high offsets 1024/1040).
Each mmap covers exactly 4096 bytes with `O_RDONLY`, RM read-only and host `PROT_READ`. The timer
advanced by roughly 20 ms after each sleep; a subsequent usermode-clock read differed by 2944 ns.
No timer writes or GPU jobs were submitted. Absence of release errors and repeat allocation
exercise the drop path; this does not claim complete QEMU hot-unplug reclamation.

The older frozen ladder refused this host's driver version before accessing hardware
(`native-unprivileged.txt`). Its guard was retained. The v3 probe is built from current source
and the host's exact measured ABI, so it does not borrow the older ladder's unchecked codec.

`matrix/*/layouts.tsv` contains compiler/DWARF measurements of `Nv01TimerMap` for each of the
30 exact supported/measured tags. Empty values/missing files are retained alongside the layout.
The reproducible full generator entry is `tools/drivermatrix/host.spec` (`host_timer_map`);
query parameters already existed in the original SDK measurements and are now consumed.

`source-map-audit.tsv` is a **textual research census**, regenerated with:

```sh
python3 scripts/bench/windows/audit_timer_map.py /workspace/ogkm-full
```

It checks the query's NON_PRIVILEGED flag using each tag's own flag definition (numeric flags
changed from `0x11` to `0x9`), zero access-right requirement, and the driver's explicit permission
for the whole timer range as `NV_PROTECT_READABLE`. All 30 tags' eight non-NVSwitch `dev_timer.h`
macro inventories have the same recorded hash. This is not a compiler proof of reachability;
it complements source reading and the actual unprivileged native run. No captured die values
feed product code. The design and full-page read-side-effect review are in
`docs/design/V3_WINDOWS_TIMER_MAPPING.md`.

The broad GPU-free run initially exposed a controller issue: `/dev/null` was a regular file,
so Cargo's rustc target-discovery subprocess read prior logs as source and cached a failure.
Trybuild then misleadingly reported every invalid snippet as compiling. Restoring `/dev/null`
to character device 1:3, mode 666, and removing this target's bad `.rustc_info.json` fixed all
nine negative samples without changing tests or snapshots. Old `/dev/null` contents were
preserved locally without publishing them. `tests.log` records **1380 tests passed, zero failed**, across 78 suites in `kf-abi`,
`kf-host`, `kf-linux-raw`, `kf-rm`, `kf-trap` and `kf-qemu`, after that repair. A negative Windows boot result would not contradict the native prerequisite.
