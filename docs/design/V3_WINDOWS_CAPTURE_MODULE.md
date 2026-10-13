# V3: the Windows GSP capture module (`tools/windows-capture/`, "kfwc")

**STATUS: DESIGN-ONLY, 2026-10-13.** The source exists, the platform independent half is tested on Linux, and the kernel driver and the companion **compile and link** here; **nothing has been loaded on Windows**, nothing was run against a GPU, and no real queue has been seen by this code. Every claim carries a label: **[M]** measured by a command run for this page, **[R]** read in source or a document, **[I]** inferred, **[U]** unverified on real hardware (a guess the design depends on, with its falsifier). An inference is never written as evidence.

Why it exists: `V3_WINDOWS_ONLY_CONSTANTS_20261013.md` (40 controls no ogkm tag defines) and `V3_CONSTANT_STRATEGY.md` s4 step 1. The decoder is `crates/kf-census`; the record format is its `docs/design/V3_GSP_CAPTURE_FORMAT.md` (KFGSPCAP v1), and **this module writes exactly that format** (section 6).

## 0. What exists, and what was and was not done

| piece | file(s) under `tools/windows-capture/` | state |
|---|---|---|
| queue discovery, walker, loss accounting, ring, record writer, sidecar (platform independent C99, no libc beyond `memcpy`/`memset`) | `lib/*.c`, `lib/*.h` | **tested on Linux** (section 11) |
| kernel driver `kfwc.sys` (legacy WDM) | `driver/kfwc_drv.c`, `driver/kfwc_ioctl.h` | **compiles (clang-cl `/W4 /WX`) and links (lld-link) [M]**; PE sanity check passes [M]; **never loaded** |
| user-mode companion `kfwc_collect.exe` | `user/kfwc_collect.c` | **compiles (mingw-w64, `-Werror`) [M]**; never run |
| install / test-signing script | `install/install.ps1` | **not parsed or run** (no PowerShell here) |
| build script | `build/build-linux.sh` | **ran end to end [M]** against a locally extracted kit |
| Linux twin of option (a) for step 0 of the validation plan | `linux-probe/mailbox_probe.c` | compiles [M]; not run (it reads a GPU) |
| observer comparison for the home-host validation | `validate/compare_with_observer.py` | tested on synthetic exports [M] |
| tests, guard, mutation check | `tests/` | section 11 |

**Prior art that this page must not hide.** The branch `origin/codex/windows-gsp-tracer-2026-10-04` (`tools/windows-gsp-trace/`, not on the integration branch) is an earlier Windows tracer: a WDM driver that **scans physical RAM** for the shared-region page table and samples the queues; per its own README it captured **4,535 valid records on an RTX 4070 under driver 580.88 / firmware 580.65.05**, with **no early prefix** (first retained queue sequence 2707/2710), and it documents a load investigation that this module reuses (section 10) [R, not re-run here]. kfwc is not a rewrite of that idea. It differs in four ways: (1) it tries the **boot-args mailbox first**, which can recover the queues from the moment they are published instead of sampling a ring that has already been used; (2) it writes the **KFGSPCAP v1** format with explicit loss records instead of KFGT; (3) the discovery, walker and loss logic is a **separate library tested against synthetic queues and kayfabe's own encoder**; (4) the read-only property is enforced by types, by a guard and by the import table, not only by review. Its scan route is the same idea as option (b) and is the only one of the two with a hardware result behind it.

## 1. What it does, in one paragraph

A kernel driver finds the NVIDIA display device in PCI configuration space, maps the first 1.1 MiB of BAR0 read-only, learns where the driver<->GSP message queues are, and on a PASSIVE_LEVEL system thread copies the raw ring entries that changed hands into a lock-free ring. A user-mode companion reads that ring with `ReadFile`, writes `capture.kfgspcap` and a sidecar `cell.json`, and the decoder (`kf-census`) does all interpretation offline. The module copies raw entries; it reads a handful of element-header fields (RPC signature and version, length, `elemCount`, `seqNum`, the checksum fold, and the CC authentication bytes) only to find where a message starts and to account for loss.

## 2. Threat model

It runs **only on our own machines**: the home Windows guest under kayfabe, and rented Windows 11 LTSC KVM VMs (windows-on-vast) where we have administrator rights, can set `bcdedit /set testsigning on`, and the GPU is passed through. There is no hostile guest here (the guest is ours and the module is meant to be the most privileged thing in it) and no attempt to survive a hostile administrator. The concerns are different:

1. **Do not disturb what we measure.** Any store to the device, any hook, any lock the NVIDIA driver could wait on, would change the traffic being recorded. Section 4.
2. **Do not collect what must not leave the machine.** No UUID, serial, VBIOS, MAC, PCI subsystem id, bus/device/function, host or user name, address, path. Section 6.
3. **Do not crash the machine under test.** A bad BAR address read, a runaway scan, an unbounded copy would turn a measurement into a blue screen. Section 9.
4. **The payload is data we must treat carefully.** Capture files contain driver message bodies that can hold board data (the format document, section 3). Capture files are host-only; only the census leaves.
5. **The device node is not a data source for other users.** Exclusive open, SYSTEM and Administrators only, write access refused at `CREATE`.

## 3. Architecture

```
 PCI config (read)  ->  find 10DE display device, BAR0, D0/decode checks          [driver, pci_*]
 BAR0 (RO map)      ->  PMC_BOOT_0, GSP MAILBOX0/1 (three registers, whitelist)   [driver, plat_read_reg32]
 system RAM (read)  ->  MmCopyMemory(PHYSICAL) of ranges inside MmGetPhysicalMemoryRanges  [plat_read_ram]
                          |
                  kfwc_engine_step()      <- worker thread, PASSIVE_LEVEL, bounded per step
                  discovery (a) mailbox -> LibOS array -> RMARGS -> init args -> page table -> 2 queues
                            (b) bounded RAM scan -> self-referencing page table -> strict validation
                  walker: per queue, copy [position, writePtr) as raw entries, account loss
                          |  KFGSPCAP records (BIND, ELEMENTS, LOSS_*, END)
                  kfwc_ring (SPSC, lock free, 16 MiB, 256 KiB reserve for BIND/LOSS/END)
                          |
 ReadFile(\\.\KayfabeWinCapture)  ->  kfwc_collect.exe  ->  capture.kfgspcap + cell.json
```

Decisions: **a legacy WDM driver, not KMDF** [reason]: no PnP, no INF, no co-installer; it is a measurement tool loaded with `sc create`/`sc start` on a machine we own, so framework object lifetime would add code (and WdfDriverStubs) for nothing; the earlier tracer loaded the same shape of driver successfully on Windows 11 build 26100 once the image flags were right [R]. **Not attached to the NVIDIA device stack**: it opens no handle to `nvlddmkm`, registers no filter, and a hung capture module cannot hold an NVIDIA IRP. **Device access by `ReadFile`, not by an IOCTL that takes parameters**: the only control codes configure the module's own behaviour (`START` with bounded numbers, `STOP`, `INFO`, `STATS`, `EVENTS`); no code takes an address from the caller.

## 4. The read-only guarantee, and how it is enforced

The claim: **kfwc never stores to device memory, to a device register, to PCI configuration space, or to any memory it did not allocate itself; it hooks and patches nothing, calls nothing in the NVIDIA driver, and reads no board-specific datum.** Four independent layers, each of which can be shown to fail:

| layer | mechanism | what shows it can fail |
|---|---|---|
| type | `kfwc_platform` (`lib/kfwc_platform.h`) has `read_ram`, `ram_contains`, `ram_range_count`, `ram_range`, `read_reg32`, `now` and **no store method**; the library cannot express a store. The mapped BAR pointer is `KFWC_MMIO` = pointer to `volatile const`; the build uses `-Wcast-qual` so a cast that drops the `const` does not compile | `tests/run_tests.sh` injects `s->bar0[off] = 1;` into the driver and requires clang-cl to refuse: "read-only variable is not assignable" [M] |
| whitelist | `kfwc_reg_read` refuses every BAR0 offset except `PMC_BOOT_0` (0x0, liveness only), `NV_PGSP_FALCON_MAILBOX0/1` (0x110040/0x110044); the driver re-checks the offset against the 1.1 MiB mapped prefix | a test asks for `NV_PGSP_QUEUE_HEAD(0)` (0x110c00), 0x110000, 0x4, 0x100000 and sees the platform never asked [M]; mutation "whitelist widened" is caught [M] |
| source guard | `tests/guard_readonly.py` fails on: `WRITE_REGISTER_*`, `WRITE_PORT_*`, `HalSetBusData*`, `MmMapIoSpace(` (non-Ex), `PAGE_READWRITE`/`PAGE_EXECUTE*`, `MmMapLockedPages*`, `ZwMapViewOfSection`, `MM_COPY_MEMORY_VIRTUAL`, hooking APIs (`PsSetCreateProcessNotify*`, `ObRegisterCallbacks`, `CmRegisterCallback*`, `IoAttachDevice*`, `IoConnectInterrupt*`), privileged stores (`__writemsr`, `__out*`), `MmGetSystemRoutineAddress`, `IoCreateDevice(` (only the SDDL-secured form), any `MmMapIoSpaceEx` without `PAGE_READONLY`, any line in `driver/` that names the mapped pointer outside an exact allow-list, any store-like member in `kfwc_platform`, any board/person word in the companion or sidecar writer (`SUBSYS`, `SerialNumber`, `UUID`, `MatchingDeviceId`, `HardwareID`, `VBIOS`, `GetComputerName`, `SetupDi`, ...) | `--self-test` injects 12 violations into a scratch copy; all 12 caught, real tree clean [M] |
| binary | the guard also reads the import table of the built `kfwc.sys`: no `WRITE_*`, `HalSetBusData*`, plain `MmMapIoSpace`, `MmMapLockedPages*`, `ZwMapViewOfSection`, no hooking import; `MmMapIoSpaceEx` must be present. Actual imports [M]: ntoskrnl `MmCopyMemory`, `MmGetPhysicalMemoryRanges`, `MmMapIoSpaceEx`, `MmUnmapIoSpace`, `HalGetBusDataByOffset` (HAL), thread/event/mutex/IoCreateDeviceSecure support, plus what `wdmsec.lib` links for its security-descriptor code | the guard's own import rule is exercised by the build (`--sys`); not mutation-tested |
| hardware | `MmMapIoSpaceEx(pa, 0x112000, PAGE_READONLY \| PAGE_NOCACHE)`: a store that got past everything above faults instead of reaching the device **[U]** (that `PAGE_READONLY` is honoured for a BAR mapping on this Windows build is the documented contract, not something observed) | first load on the home guest: a deliberate fault test is NOT included (it would be a store); the observation is that `MmMapIoSpaceEx` returns non-NULL for `PAGE_READONLY \| PAGE_NOCACHE` |

What the guarantee does **not** cover: the module reads three registers; **a read of a GPU register could in principle have a side effect** [U]. For these three (`PMC_BOOT_0` is the chip-id word; the falcon mailboxes are plain scratch registers) none is documented as clear-on-read in the open headers [I]; the Linux twin `linux-probe/mailbox_probe.c` reads the same three through a `PROT_READ` mapping and is validation step 0. A read of an unpowered or decode-disabled device returns all ones or hangs on some platforms: the driver therefore reads PCI command (memory decode on) and the power-management capability (D0) before **every** register read, and refuses a BAR value that has not been stable for 20 ms. Reading RAM with `MmCopyMemory` is documented as safe for physical addresses inside RAM ranges; the module only passes addresses it has checked against `MmGetPhysicalMemoryRanges`, never an MMIO address.

The module reads the PCI **vendor, device, revision, class, header type, command, status, capability list, BAR0/BAR1** of the NVIDIA device it selects, and **vendor ids of other functions while scanning for it**. It never reads the subsystem ids (0x2C), the expansion ROM BAR, or the device serial number extended capability. Only vendor/device/revision leave the driver (`INFO`), and bus/device/function never does.

## 5. How it finds the queues

### 5.1 What kayfabe's own FSM tells us about the chain [R]

`crates/kf-gsp/src/boot.rs` (`GspFsm::publish`, E6) is the faked GSP's side of the same handshake the real driver performs, so it is the specification of the chain that a passive reader can follow:

1. The driver writes the physical address of the **LibOS init-args array** into `NV_PGSP_FALCON_MAILBOX0` (low) / `MAILBOX1` (high) on the Turing..Ada regime (`ogkm-580: kernel_gsp_tu102.c kgspProgramLibosBootArgsAddr_TU102`; offsets 0x110040/0x110044 at `ga102/dev_gsp.h:27,29` and republished for Blackwell at `blackwell/gb100/dev_gsp.h:27,30`; `tools/vfio-gsp-observer/layout-580.json`). **Hopper and later (FSP regime) do not publish the address this way**: the address names `GSP_FMC_BOOT_PARAMS` through an indirection (`BootSequence::boot_args_indirection`, `V3_FAMILY_PORT_BLACKWELL.md`) and `MAILBOX0` must read 0. Option (a) therefore applies to TU10x..AD10x; for everything else it reports `ZERO` and option (b) is the only route.
2. The array is 4096 bytes of 32-byte entries `{id8 @0, pa @8, size @16, kind u8 @24, loc u8 @25}` (`libos_init_args.h:31-56`, identical at ogkm 580 and 610); the entry with id8 `"RMARGS"` (value `0x0000524d41524753`, built most-significant byte first) points at `GSP_ARGUMENTS_CACHED`. A zero entry is *skipped*, not a terminator.
3. Its first member `MESSAGE_QUEUE_INIT_ARGUMENTS` holds, at 580: `sharedMemPhysAddr u64 @0`, `pageTableEntryCount u32 @8` (4 pad bytes), `cmdQueueOffset u64 @16`, `statQueueOffset u64 @24`; at 610 five more fields follow (`queueElementHdrSize @32`, ...) [R, `kf_abi::versions::GspInitArgsWire`].
4. The shared region's **page 0 is a table of one `u64` physical address per 4096-byte region page, entry 0 naming the table page itself** (`GspMsgQueuesInit`; `RegionMap::load`); the command and status queues are `cmdQueueOffset`/`statQueueOffset` bytes into the region. The status queue's tx header can still be all zero before the firmware starts.
5. A queue is a `msgqTxHeader` (eight u32: version 0, size, msgSize 4096, msgCount, writePtr, flags = `SWAP_RX`, rxHdrOff 32, entryOff 4096) followed by `msgCount = size/4096 - 1` entries; a message occupies `elemCount` entries (580: u32 at element offset 40; 610: derived from `rpc.length`), element header 48 bytes (580) or 16 bytes with MCTP/NVDM words (610) [R, `ring.rs`, `element.rs`, `kf_abi::versions::GspElementWire`].

The pinned ogkm tree is not checked out in this worktree (`third_party/` holds only `README.md`) [M]; every constant above is therefore taken from kf-gsp, kf-abi, kf-chip and `tools/vfio-gsp-observer/layout-580.json`, which cite ogkm line numbers. The census oracle independently re-derives message framing from the same sources.

### 5.2 Option (a): boot-args mailbox [U]

`kfwc_discover_mailbox`: read `PMC_BOOT_0` (must not be all ones), read `MAILBOX0/1`; classify (`UNREADABLE`, `ZERO`, `SUSPENDED` = `0x80000000`, `BAD_ADDR`: not page-aligned or not inside a RAM range); read the 4096-byte array; find `RMARGS` and require `kind == contiguous`, `loc == sysmem` (the observer's own conditions); read the 40-byte args structure (it must lie inside one page); require `pageTableEntryCount` in 3..512, offsets below 4 GiB; load the page table and require every entry nonzero, page aligned, **inside a RAM range**, unique, and entry 0 equal to the table's own address; validate both queue headers with the `msgqRxLink` predicate specialised to the one geometry the driver creates (flags exactly `SWAP_RX`, `msgSize` = `entryOff` = 4096, `msgCount = (size - 4096)/4096`, `writePtr < msgCount`, sizes 8 KiB..1 MiB). Every refusal has a named status; tests cover each one [M, `mailbox_failures`: 17 distinct corruptions].

**What is unverified, and it is the whole point of the option:** *whether `MAILBOX0/1` still hold the address after the GSP has booted, on real hardware, under Windows.* The mailbox is written once before the boot (`kgspBootstrap_TU102`) and the suspend path later puts `0x80000000` in `MAILBOX0` [R]; nothing read here says what the firmware does to the registers in between [U]. **Falsifier:** run `linux-probe/mailbox_probe.c` on a Linux box with the NVIDIA driver loaded (step 0) and the module on the first Windows rental; if the pair reads zero or a non-address while the queues exist, option (a) only works if the module is loaded before the NVIDIA driver starts and polls (it polls every 10 ms while searching), and otherwise the design falls back to (b) with no loss of correctness. Whenever (b) attaches first, the engine runs the mailbox chain once as a **cross-check** and records the outcome (`EV_XCHECK`: status, match yes/no) in the sidecar, so the first real capture answers the question for that die and driver for free.

### 5.3 Option (b): bounded RAM scan, strict validation [I from the earlier tracer's hardware result]

One RAM range is read in chunks of at most `scan_bytes_per_step` (default 1 MiB, hard maximum 1 MiB) per engine step; between steps the worker sleeps `scan_period_us` (default 2 ms, minimum 0.5 ms), so the scan reads at most about 500 MB/s and touches **only** addresses inside `MmGetPhysicalMemoryRanges`. A page is a candidate when its first `u64` equals its own physical address and the second is a nonzero page-aligned address. A candidate is accepted only if: entry 1 points at a valid command-queue header (the full `msgqRxLink` specialisation above), the status queue's header (at `1 + cmd_pages`) is valid **or all zero** (not yet published), every entry up to `1 + cmd_pages + stat_pages` is nonzero, aligned, inside RAM and unique, and the two queues do not overlap. Tests: a self-reference with junk, a self-reference to a non-queue, a real-looking table with a bad flags word, a pointer outside RAM, an unaligned and a zero address are all refused [M]; the scan step reads at most 256 KiB per step when configured so [M]; the scan does not start while the mailbox reads zero until `scan_delay_ms` (default 5 s) has passed, and starts at once if BAR0 is unreadable or the mailbox holds garbage [M].

Limits, all inherited from the idea: **the scan finds a queue that already exists**, so it recovers only what is still in the ring (the walker's backward chain reconstruction recovers up to the full ring, 63 entries of history, and announces the unseen prefix); it assumes **DMA addresses equal CPU physical addresses** (no IOMMU remapping of the GPU's DMA) [U on rentals, true under kayfabe]; one page of page-table entries (512 pages) only, chained tables are refused by name; confidential-compute data is copied raw and left for the decoder; a stale allocation from a previous driver life can match until the new one appears (the mapping check detaches when the page list changes).

## 6. What is recorded: KFGSPCAP v1, aligned exactly

`lib/kfwc_record.[ch]` implement the format of `docs/design/V3_GSP_CAPTURE_FORMAT.md` (branch `claude/gsp-census-20261013`, commit `e2a4618bb`, `crates/kf-census/src/capture.rs`): a 128-byte file header (`KFGSPCAP`, version 1, header sizes 128/32, `os_id` 1, `module_version`, `pci_vendor_id`, `pci_device_id`, `pci_revision_id`, `ts_hz`, `ts_start`, `driver_version[32]`, `decode_tag[32]`, reserved bytes zero) and records of a 32-byte header (`"KREC"`, `kind`, `dir`, `queue_id u16`, `payload_bytes`, `counter`, `ts`, `first_entry`, `entry_count`) plus payload: `BIND` (the queue's raw 32-byte `msgqTxHeader`), `ELEMENTS` (`entry_count` consecutive raw ring entries from `first_entry`, split into two records at the ring end), `LOSS_OVERFLOW`/`LOSS_UNREADABLE`/`LOSS_UNKNOWN` (kinds 3-5), `END`. The module copies raw entries; **large controls arrive split across continuation records and the module does not join them**, the decoder does. `queue_id` is the discovery generation (a re-created queue gets a new id and a new `BIND`).

**Agreement is checked, not asserted [M]:** (1) the record and file-header byte layouts are tested against the field tables of `capture.rs`; (2) the output of the module's engine, fed with bytes from kayfabe's own encoder (`kf_gsp::encode_message`, `TxHeader::encode`) through synthetic rings, is **accepted by the real `kf-census` binary** for both element layouts (`oracle: PASS`, 20 command + 20 status messages each, tags 580.82.07 and 610.43.02); (3) a deliberately overrun run is **rejected** by `kf-census` with `capture_loss` and "87 missing" - the number the engine independently counted. Where the two crates disagree, `capture.rs` wins and the module changes.

Not in the capture stream (the format has no field for them): discovery events (START, ATTACH with method/layout/geometry, DETACH, the mailbox cross-check). They are kept in a 64-slot event log in the driver and written to the sidecar `cell.json`. The sidecar's key set is fixed by an allow-list (`kfwc_meta_allowed_keys`) that a test checks against the output, including a hostile driver-version string that tries to inject a `"uuid"` key [M]. The companion reads two registry values for the cell (`ProviderName` to find the NVIDIA display-class instance, then its `DriverVersion`), the Windows `CurrentBuildNumber`, `UBR` and `DisplayVersion`. It does not read `MatchingDeviceId`, hardware ids, `DriverDesc` or anything under the device's enumeration key.

## 7. Loss: how it is detected, and the correction about sequence numbers

**Correction folded in (coordinator, 2026-10-13).** The decoder measured `rpc.sequence == 0` in all 70,033 messages of the five Windows 580.88 captures, so **`rpc.sequence` is never used by this module**. There are three different numbers and only two are used:

| number | where | used by kfwc |
|---|---|---|
| `rpc.sequence` | RPC envelope @ +24 of the envelope | **never** |
| ring pointers (`writePtr` in the producer's `msgqTxHeader`, `readPtr` in the rx header) and the raw entry index | queue header / `first_entry` | **yes, primary**: the engine copies exactly `[position, writePtr)`; a position whose first entry no longer starts a message is a re-synchronisation, announced |
| the **transport element `seqNum`** (@ +36 in the 580 element header, one number per *message*, per queue instance) | element header | **yes, secondary**: the census doc records it [M] "starts at 0 for each queue instance in every one of the five captures, no `prefix_missing`, no `seq_gap`" (`V3_GSP_CAPTURE_FORMAT.md` s5.3, s6); it is also what `kf-gsp` maintains on both sides (`rxSeqNum`/`txSeqNum`) |

Pointers alone cannot see a producer that lapped the observer by a whole number of rings (63 entries); `writePtr` is modular. That is the case the `seqNum` exists to catch, and it is the only reason the engine reads it. **Falsifier [U]:** if a real capture shows the transport `seqNum` is not continuous (a legitimate jump), the engine would announce phantom `LOSS_UNKNOWN` records and degrade towards copying only the newest message per poll; the first real run's `stats.seq_gap_messages` and the decoder's own `seq_gap` would show it at once. The mitigation, if needed, is a pointer-only mode; it is not built because the measurement says the problem should not occur.

Guarantees, each tested [M]: every entry the engine did not copy is announced by a LOSS record in stream order **or** is visible as a hole in the record `counter` (the counter is consumed before the sink is offered the record, so a refused record leaves a hole even if no loss record can be stored). Specifically:

| event | record | notes |
|---|---|---|
| attach to a stream whose first copied message has `seqNum != 0` | `LOSS_UNKNOWN` (count unknown) before it | "prefix unseen" |
| producer lapped us / seq jumped / re-synchronised after an unreadable entry | `LOSS_UNKNOWN`, exact message count kept in `stats.lapped_messages` / `seq_gap_messages` | the decoder's `seq_gap` count matched the engine's count (87) in the lossy run |
| entry unstable after a retry (torn copy) | none if the retry is clean; after 8 polls re-synchronise, the message is counted through the seq arithmetic, announced once | `LOSS_UNREADABLE` when the engine knows the range |
| queue header invalid, queue re-created, page list changed, 8 failed header reads | `LOSS_UNKNOWN` for each bound direction, then a new generation | clean `STOP` writes none |
| output ring full | `LOSS_OVERFLOW` with the **exact entry range**, written before the next stored record | priority records (BIND, LOSS, END) use a 256 KiB reserve that data records cannot touch |
| the reader never drains | same as above, indefinitely; never blocks the worker | |
| confidential-compute authentication bytes | the raw entries are copied; nothing is dropped | the decoder says what they are |
| unparseable history at first sight (write pointer non-zero, no intact message ends there) | `LOSS_UNKNOWN` | not silently skipped |

What it can still get wrong without a marker, [I] and not covered by a test that exercises real ring behaviour: an entry rewritten between the header peek and the verified copy that still folds to zero (retry plus checksum make this negligible, not impossible); a driver that re-uses a ring and restarts its `seqNum` at 0 without changing the queue header (handled as "sequence went backwards": a `LOSS_UNKNOWN` and a fresh prefix, tested only with synthetic data); everything before the module attached is announced as an unseen prefix, not recovered.

## 8. What it cannot see

- Anything that is not in the two shared-memory rings: BAR0 register traffic, doorbells, interrupts, DMA to/from the GPU, the display engine's own pushbuffers, VRAM, the PCI config traffic. The census list's group D rows (behaviour and timing of the display engine) are out of reach by design.
- Timing finer than the poll (1 ms requested; the actual Windows timer resolution is requested with `ExSetTimerResolution`, not guaranteed [U]); timestamps are "when the bytes were observed", not when the driver wrote them, and not an ordering key between the two queues.
- Any message before attach (section 5.3); the early boot burst is the part most at risk, which is why load order matters (section 12).
- A driver that does not use GSP (`nvidia-smi -q` shows "GSP Firmware Version: N/A": the earlier tracer saw exactly that once [R]): there are no queues.
- Hopper and later through option (a); multi-GPU machines (the first display device or `--gpu N`, one at a time); PCI segments other than 0 (the scan uses `HalGetBusDataByOffset`, bus 0..255).
- Windows' own Hyper-V/VBS isolation: with HVCI on, test-signed drivers still load but `MmMapIoSpaceEx` is the HVCI-compatible mapping API; with VBS isolating the GPU nothing here has been tried [U].

## 9. Driver internals worth knowing

- **Threads and IRQL**: one system thread at `LOW_REALTIME_PRIORITY`, PASSIVE_LEVEL everywhere; the only waits are `KeWaitForSingleObject` on the stop event with a timeout (`poll_us` default 1000, `scan_period_us` default 2000). One step is bounded: 1 MiB of scan, 64 messages per queue; at most 16 consecutive productive steps without sleeping.
- **Locks**: the worker takes a fast mutex around each engine step, only so that `INFO/STATS/EVENTS` can read a consistent copy; the ring between the worker and the reader is lock free (release/acquire, one producer, one consumer; ThreadSanitizer-clean in a 200,000-record stress run [M]). Nothing here takes a lock the NVIDIA driver can wait for.
- **Memory**: one 16 MiB non-paged ring, one 1.2 MiB non-paged engine (scratch pages included, so nothing large is on the kernel stack), allocated at load; freed at unload after the worker is joined. The engine never allocates.
- **Lifetime**: exclusive device, `START` resets the stream (record counter 0, fresh ring), closing the handle stops the worker, `STOP` writes `END` and leaves the records readable. `Unload` joins the worker before freeing.
- **Timer resolution** is raised on `START` and released on stop.
- **BAR0 handling**: the BAR value is re-read from config space for every register read; a changed value unmaps, and a new value is mapped only after 20 ms of identical reads (a BAR-sizing write by the PnP manager must not become a mapping). Reads are skipped while memory decode is off or the power state is not D0.
- **Build flags**: those of the earlier tracer's load investigation (`/tsaware:no`, `.retplne` readable, nonpageable code and data sections, `/integritycheck`, GS cookie); `tests/check_driver_pe.py` (copied from that branch) rejects the image shapes that failed to load there.

## 10. Build route (investigated, measured)

**Verdict: the driver cross-compiles on this Linux box [M], with Microsoft's NuGet WDK headers, clang-cl and lld-link; no Windows machine is needed to produce `kfwc.sys`. Whether the Linux-linked image *loads* is the earlier tracer's open question and remains one [R].**

- Tools present: clang-cl 21.1.8, `lld-link-19`, `x86_64-w64-mingw32-gcc` (companion), python3 [M].
- Kit: `Microsoft.Windows.WDK.x64` and `Microsoft.Windows.SDK.CPP` **10.0.28000.2526** from `api.nuget.org`. Download sizes [M, HTTP Content-Length]: 113,683,738 B and 160,886,794 B (about 275 MB). **The SHA-256 pins in `build/build-linux.sh` (taken from the earlier tracer) were re-verified against the live files by streaming both through `sha256sum` without writing them to disk [M]: both match.** Disk on this machine was too small for 275 MB, so for this session only the needed members were fetched with HTTP range requests (4.1 MB + 3.0 MB transferred, 37 MB extracted: `km` and `shared` headers, the `ucrt` headers, and four libraries `ntoskrnl`, `hal`, `wdmsec`, `BufferOverflowK`); `build/build-linux.sh` then ran end to end against that tree via `KFWC_KIT=` and produced `kfwc.sys` (43,520 B, sha256 `ce2c9603...e502ae`, byte-identical on a second link) and `kfwc_collect.exe`. Everything stays under `/tmp`; nothing from the kit is in the repository.
- **Licence [R, read from the package's `LICENSE.txt`]:** "MICROSOFT PRE-RELEASE SOFTWARE LICENSE TERMS - WINDOWS HARDWARE LAB KIT (HLK) for Windows 11 / WINDOWS DRIVER KIT (WDK) for Windows 11". Section 1a grants the right to "install and use any number of copies of the software on your premises ... to design, develop and test your programs (e.g., software, drivers, tools, etc.) for use with Windows operating systems", and not to use it "in a live operating environment" without another agreement; there is a confidentiality clause (section 7: five years, with exclusions for public information) and the term is "until commercial release". The nuspec has `requireLicenseAcceptance=true`. So: building a measurement driver for our own machines is within the stated use; **redistributing the kit or its headers is not**, which is why the script downloads it per user and keeps it out of git; the SDK.CPP package points at `https://aka.ms/WinSDKLicenseURL` (not fetched). This is a pre-release licence on a pre-release kit number; a decision on whether to pin a released kit instead is in section 13.
- **Fallbacks**, in order: (1) build on a Windows VM or on the rental with the installed WDK/MSVC (`msbuild`; the earlier branch has `build.cmd` and a GitHub Actions workflow `windows-gsp-build.yml` with `build-msvc-ci.ps1` that produced loadable images when the Linux-linked one was rejected [R]); (2) use the MSVC artifact for the first load and the Linux build only after "runtime parity" is shown. `windows-drivers-rs` (Rust) and `xwin` were **not tried**: both need the same WDK headers/libs from the same package for kernel mode (xwin supplies the user-mode SDK and CRT, not `km`) [I], and the C route already works, so they add dependencies without removing the Microsoft kit.
- Code signing: the build output is **unsigned**; `install/install.ps1` creates a throwaway self-signed certificate on the target, imports it into `Root` and `TrustedPublisher` (by `X509Store.Open(ReadWrite)`, because `Import-Certificate` returned `E_ACCESSDENIED` on a fresh image [R]), and signs with `signtool`. Requires test signing on (Secure Boot off).

## 11. Tests that run on Linux (no GPU, no Windows)

All from the repository root; scratch output goes to `/tmp` and was deleted at the end of the session.

```sh
KFWC_KIT=/tmp/kfwc-wdk KFWC_CENSUS_BIN=/tmp/kfwc-target/debug/kf-census \
    tools/windows-capture/tests/run_tests.sh          # everything below; exit 0 only if all pass
make -C tools/windows-capture/tests test               # just the library tests (ASan + UBSan)
python3 -I tools/windows-capture/tests/guard_readonly.py [--self-test]
python3 -I tools/windows-capture/tests/mutate.py
```

Results of the last run [M] (`ALL STEPS PASSED`):

| test | what it covers | result |
|---|---|---|
| `test_main` (ASan+UBSan) | 26 tests: KFGSPCAP record/file-header byte layout and strictness; element peek/checksum for both layouts and CC bytes; mailbox discovery success and 17 corruptions; scan signature false positives (junk, wrong queue, bad flags, outside RAM, unaligned, zero); walker on both layouts, late attach with prefix marker, 400 messages through a wrapping ring with runs split at the ring end, a producer lapping the observer (63 retained of 150, count exact), sequence gap, 32-bit wrap-around, empty ring then messages, 30 messages of history, unpublished data, per-step budget, status header published late, scan rate limit (<= 256 KiB per step) and gating by the mailbox, cross-check events, detach/re-attach with new `queue_id`/`BIND`, page table rewritten, failing reads, corrupt entry mid-range, a torn copy that heals, one that never heals, CC bytes copied raw, sink full announced before the next record, read-only surface (only whitelisted registers touched), layout auto-detect and a wrong pin, `END` | **26/26 tests, 325 checks** |
| `test_ring` | whole-record delivery, reserve for priority records, 200,000-record two-thread stress | **51 checks**, clean under ASan/UBSan **and** ThreadSanitizer |
| `test_meta` | sidecar key allow-list (hostile version string cannot add a key), escaping, banned words absent, no silent truncation | **21 checks** |
| `test_kayfabe_roundtrip` + `kf-fixture` | the C replica of the encoder equals `kf_gsp::encode_message` byte for byte (40 messages, 9 payload sizes up to the 16-entry maximum, both layouts); `TxHeader::encode` output equals the synthetic producer's header and is accepted by discovery; kayfabe's runs placed raw in both rings come out identical; the files it writes | **221 checks**; `kf-census` on the output: **PASS** for 580.82.07 (pre-610) and 610.43.02, and a lossy file is **rejected** with `capture_loss` and 87 missing = the engine's count |
| `test_compare.py` | `validate/compare_with_observer.py`: clean match, a flipped byte, a module-only message | 5 checks |
| `guard_readonly.py --self-test` | 12 injected violations | **12/12 caught**, real tree clean |
| `mutate.py` | 18 deliberate library bugs (inverted seq compare, no ring-end split, loss never announced, hole-hiding counter, checksum off, widened register whitelist, no priority reserve, ...) | **18/18 caught** (one survived the first version of the tests, `ring wrap not split`, and the missing invariant was added) |
| `build-linux.sh` | clang-cl `/W4 /WX /clang:-Wcast-qual`, lld-link, PE check, import guard, mingw companion | OK |
| negative compile | store through the mapped pointer | refused by the compiler |

Not tested, and why: the Windows API behaviour of `kfwc_drv.c` (no Windows), `install.ps1` (no PowerShell here), `kfwc_collect.exe` (never run; wine exists but a prefix would not fit the disk and could not exercise the driver anyway).

## 12. Validation plan (not run). Each step has a falsifier.

**Step 0, any Linux box with the NVIDIA driver loaded** (home host or a Linux rental; root; read-only): `linux-probe/mailbox_probe.c /sys/bus/pci/devices/<bdf>`. Question: does option (a) lead from the mailbox pair to the queues while the driver is running, on this die? *Falsifier:* `status != OK` with a non-zero pair means the pair is stale or the chain differs; zero means the pair is cleared after boot (then (a) is only for a module loaded before NVIDIA). Also read the mailbox three times a second apart: three identical values support "stable"; this is also the cheapest test that reading the three registers has no visible side effect on a live driver (`nvidia-smi` unaffected, no Xid in `dmesg`).

**Step 1, home Windows guest under kayfabe, module only** (kayfabe emulates BAR0 and keeps the GSP queues in guest RAM; its register model serves the boot-args pair from a shadow [R, `boot.rs` `mailbox_lo/hi`], so option (a) should *work by construction* here - this step checks the **code path**, not the hardware assumption). Procedure: `install.ps1 -Action Preflight`, `EnableTestSigning`, reboot, `Install`; `kfwc_collect.exe C:\traces\r1 --seconds 600 --decode-tag 580.82.07` started **before** the NVIDIA driver is enabled (device disabled in Device Manager, enabled after the collector runs). *Falsifiers:* the module does not load (record `sc start` error, the NtLoadDriver status, and the CodeIntegrity log; the earlier tracer's `C0000018` story is in its `LOAD_TESTS.md`); `INFO` shows `bar_state != MAPPED`; no `ATTACH` event; `kf-census` on the capture reports anything but the losses the module announced.

**Step 2, same boot, byte for byte against kayfabe's observer.** Run the guest with `WIN_GSP_OBSERVER=1` (`scripts/bench/windows/windows_broker.sh` -> `kf3-gpu.x-gsp-observer=$RUN/gsp.jsonl`) while the module runs; then `validate/compare_with_observer.py capture.kfgspcap gsp.jsonl`. *Falsifier:* **any** message the module recorded that the observer did not record with identical bytes (`module_only > 0` or `byte_conflicts > 0`) means the module's copy is not the driver's message and it is not used. Observer-only messages are expected for the prefix before attach and for anything the module announced as lost; count both. Also compare against a boot with the module **not** loaded: no new TDR, no timing change beyond what the observer already causes (the observer itself perturbs: it copies up to ~4 MiB under the BQL [R]).

**Step 3, decoder agreement.** `kf-census --ogkm-tag <tag> capture.kfgspcap` and the observer's file give the same per-control census over the common range. *Falsifier:* a differing `requests`/`replies`/`paired` for any control id within the common sequence range.

**Step 4, the first cheap rental, before any measurement.** In this order, stopping at the first failure: (1) `install.ps1 -Action Preflight`: **is test signing allowed (Secure Boot off, `bcdedit` accepts it)?** The windows-on-vast notes say test signing can be enabled; confirm on this image. Is VBS/HVCI on? Is the GPU the only NVIDIA display device? Is GSP in use (`nvidia-smi -q`: firmware version not `N/A`)? (2) Load the driver **without** the NVIDIA driver enabled: does `kfwc.sys` load with a Linux-linked image? If not, use the MSVC-built image (section 10). (3) `kfwc_collect --status`, then a 60 s capture with the GPU idle: `INFO.bar_state`, `gpu_count`, `pci_device_id` match the rental's GPU? (4) Enable the NVIDIA driver while a capture runs: **does discovery find the queues, by which method (`ATTACH` event, `method` 1 = mailbox, 2 = scan), and what does the mailbox cross-check say?** (5) Repeat with the module loaded *after* boot: does the scan find the live queues (the earlier tracer did) and how many messages are retained? Record the cost: scan time per pass for the machine's RAM, CPU use of the worker.
*Falsifiers:* (4) neither method attaches while `nvidia-smi` works: the layout assumption (RAM addresses = DMA addresses; single PTE page) is wrong on that machine, and the capture is abandoned there; (5) `LOSS_UNKNOWN` records in an idle second: the transport `seqNum` is not continuous (section 7).

**Step 5, repeat on a second die or driver version** before any cross-die statement. Nothing here supports a per-family rule (the strategy's section 3).

## 13. Risks and decisions for the owner

1. **Hardware assumptions, all [U]:** the mailbox pair survives boot; reading the three registers is harmless; `PAGE_READONLY` BAR mappings are accepted; physical == DMA addresses on the rentals; the Linux-linked `kfwc.sys` loads. Each has a step above that answers it; none is answered by anything run so far.
2. **Disk-space discipline during development:** the dev box had under 1 GB free; the 275 MB kit had to be fetched partially. Not a risk to the product.
3. **Provenance.** The module is MEASUREMENT per `V3_PROVENANCE.md`: it reads shared memory and three registers and copies bytes; its decode comes from ogkm-derived layouts already in kayfabe. No closed binary was examined for this work. The branch above contains, in its *own documentation*, offsets into a closed driver; this page cites none of them and none was carried into `tools/windows-capture/`. **Decision for the owner:** whether the earlier branch is merged as is (its docs contain that material) or only the load-investigation facts are restated.
4. **Licence of the build kit** is a pre-release Microsoft licence (section 10). **Decision:** accept it for local builds, or pin a released kit (a different NuGet version needs new pins and a re-run of the link recipe).
5. **Format coupling.** `lib/kfwc_record.[ch]` duplicate the field tables of `capture.rs`; a change there must change here (tested by `kf-census` on our output only when `KFWC_CENSUS_BIN` is set). **Decision:** vendor a golden file into `crates/kf-census/tests` produced by this module's round trip.
6. **Scan cost and perturbation** on a 16-GB-plus rental are unmeasured; defaults are conservative (at most about 500 MB/s) but the scan repeats pass after pass for as long as nothing attaches, e.g. on a rental whose NVIDIA driver is not installed yet; a slow-down after the first pass was not built.
7. **Sidecar and file contents:** capture files can hold board data inside message bodies (format doc s3). The module cannot prevent it; the rule is host-only files. **Decision pending upstream:** scrub stage or allow-list before anything leaves the rental.
8. **Multi-instance or restarting drivers:** a `START` resets the stream; a driver restart in the middle of a capture is a new generation (announced), not a new file.

## 14. Falsifier summary

| claim | falsified by |
|---|---|
| the module never stores to the device, PCI config or foreign memory | any guard violation; a non-NULL store import; a compile of `s->bar0[x] = 1`; first-load behaviour of the mapping |
| its output is the KFGSPCAP v1 format | `kf-census` rejecting a file it wrote (it accepts both layouts today [M]); a byte-layout test failing against `capture.rs` |
| the engine's loss accounting is exact for lapped rings | the decoder's missing-message count differing from `stats.lapped_messages + seq_gap_messages` (87 = 87 today [M]) |
| every loss is visible | a hole in `counter` with no LOSS record near it, or a decoder `seq_gap` outside any announced loss |
| module bytes equal the driver's bytes | step 2: any `module_only` or `byte_conflict` against the observer |
| option (a) works on real hardware | step 0 / the cross-check event: `status != OK` while queues exist |
| option (b) works on rentals | step 4 (5) |
| the walker is only as exact as the transport `seqNum` is continuous | a real capture with `stats.seq_gap_messages` > 0 and no loss in the decoder |
