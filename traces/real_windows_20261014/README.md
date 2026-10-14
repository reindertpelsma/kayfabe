# kfwc on a real GPU under the real Windows NVIDIA driver (2026-10-14)

**STATUS: LIVE, 2026-10-14 (evidence for `docs/design/V3_WINDOWS_CAPTURE_MODULE.md` section 12b).**
Labels: **[M]** measured by a command in this directory, **[I]** inferred, **[U]** not measured.
Capture files (`*.kfgspcap`, 3 MB to 96 MB each) stay on the trusted host (`/data/kfwc-real-20261014/`,
sha256 in `capture_sha256_host_only.txt`); only census text, sidecars and scripts are here. No UUID, serial,
VBIOS string, MAC or PCI subsystem id is in any file (checked with grep; the one VBIOS-bearing registry
value the session printed was not saved).

## The box

Vast rental (own id, destroyed at the end): KVM VM, Windows 11 Enterprise LTSC Evaluation 26100.1742,
RTX 4070 Ti SUPER (PCI device 0x2705 = AD103, rev 0xa1), NVIDIA Windows driver 32.0.15.8088 (580.88),
51,554 MiB RAM, Secure Boot **off** (`Confirm-SecureBootUEFI` returned false), VBS/HVCI 0/0, one NVIDIA adapter.
`preflight_default.txt` is the install script's Preflight. `bcdedit /set testsigning on` + one reboot was
needed (it was off) and worked. Build: `build-linux.sh` on the Linux host, `kfwc.sys` sha256 `9b80361a...a4f1`
(44,032 B) and `kfwc_collect.exe` `b7732d2e...7acf`, **byte-identical to the hashes in the design doc** [M];
`install.ps1 -Action Install` signed (`Set-AuthenticodeSignature`, Valid), `sc start` returned RUNNING at the
first attempt; the Linux-linked image loads on real hardware [M]. No bugcheck.

## Finding 0 (changes everything below): the stock GeForce Windows driver does not use GSP

`nvidia-smi -q` showed `GSP Firmware Version : N/A`; neither `EnableGpuFirmware` key existed (adapter class
key `...\{4d36e968-...}\0000` and `Services\nvlddmkm`). In this default configuration **there are no queues**:
captures A, A2, B (`default_*`): 0 attaches, 0 messages. After setting `EnableGpuFirmware=1` in both keys
(registry only; no GPU write) and a `pnputil /restart-device`, `nvidia-smi` shows GSP firmware 580.65.05 and
queues exist (captures C-F, `gson_*`). The previous agent's note ("stock GeForce Windows driver is monolithic RM
by default") is confirmed on real hardware [M].

## Measurements, each with the falsifier stated before the run

Runs: **A** (default config, mailbox + scan, 240 s, one device restart), **A2** (45 s idle), **B**
(`--no-mailbox --no-backoff`, 900 s, one restart), all GSP off. **C** (GSP on, mailbox + scan, 150 s, restart at
t+~20 s, trace variant 1), **D** (GSP on, started in steady state, 60 s), **E** (trace variant 2, 240 s, restarts at
t+~21 s and t+~112 s), **F** (`--no-mailbox --no-backoff`, 420 s, restart at t+138 s), **H** (CPU, `--no-mailbox`).
C, D use `mbx_trace_v1.patch` and E, F, H use `mbx_trace_v2.patch`: **diagnostic variants of the engine** (read-only
guard clean, the companion is unchanged; v1 logs each change of the mailbox status while searching, v2 additionally keeps
reading the two whitelisted registers after attach and logs each change of the raw pair as event 6). Their `kfwc.sys`
hashes differ from the pinned one (v1 `b0cf6547...`, v2 `46057edf...`); A, A2 and B ran the **pinned binary**
(`9b80361a...`). The patches only add events; attach, walking and record writing are untouched.

| # | question | falsifier | result |
|---|---|---|---|
| 1 | Is the BAR0 mailbox pair (0x110040/0x110044) readable after GSP boot under the Windows driver? | pair reads all-ones, a fault, or the register read hangs; or the pair is non-zero and chains to the queues while GSP is idle | **Readable: yes** (BAR0 state MAPPED; 20,959 mailbox reads in A, no all-ones in steady state, no hang). **Holding the address: no, except for about 0.15 s during the GSP boot.** In steady state the pair is exactly 0:0 (status ZERO) on every sample, in D (late start, minutes after boot) and in E/C for the ~90 s after boot [M]. On each driver restart the pair went 0 -> 0xfe (C only) -> all-ones (BAR unreadable) -> 0 -> **the boot-args address** -> 0 (E gen 2 `cell_timeline.txt`: valid 22.463 s, zero again 22.617 s; gen 3: 113.523 s -> 113.679 s: 154 and 156 ms between first valid and first zero sample, 10 ms sampling) -> a one-sample 0x0000000c_00000000 at +0.42 s (gen 2) -> 0 for the rest of the run. Option (a) therefore works only if the module polls while the GSP boots; a 10 ms poll caught the window in 3 of 3 restarts (C, E gen 2, E gen 3) [M]. |
| 2 | Does option (a) attach to both queues, from message 0? | decoder `prefix_missing` or first `seqNum` != 0 on an instance attached by the mailbox | **Yes, 3 of 3 [M]**: C (the whole boot, `kf-census`: PASS, 4,331 cmd + 4,337 status, 0 anomalies), E gen 2 (4,992 + 5,004) and E gen 3 (4,305 + 4,312) all start at `seqNum` 0. Command queue BIND ~1.6 s after the restart began; the status queue BIND comes about 0.22 s after the command queue's. |
| 3 | Does option (b) (RAM scan) find both queues, and in how many seconds on this RAM? | scan never attaches on a live queue within one pass, or attaches to a stale allocation | **Yes, 4 of 4 attaches [M]**: F gen 1 at **8.1 s** after start, F gen 2 (the queue the restart created) **8.4 s** after the detach (the new queue exists ~1.6 s after the restart), D at 25.6 s (includes the module's 5 s delay and a slower scan, below), E gen 1 at 20.2 s. In all four the queue sat at about 3.3 GB into the scan order, i.e. in the same low RAM (the allocation was at the same place before and after the restart in F). Scan rate: **398 MB/s** with `--no-mailbox` (B: 358.7 GB in 900 s, 6 completed passes plus 0.49 -> a pass is about 139 s over ~55 GB) but **150-165 MB/s** whenever the mailbox poll is on (A 156, A2 151, D 161 MB/s): the full pass with mailbox on would be about 350 s [I, 55 GB / 156 MB/s]; the cause (the 10 ms mailbox read: PCI config reads before every register read) is [I]. The worst case, a queue at the top of RAM, was not measured. A scan attach is late: F gen 1 first seen at `seqNum` 4,678 (command) / 4,685, F gen 2 at 3,221 / 3,225, D at 4,852, E gen 1 at 5,300: **3,000-5,300 messages per direction are unseen and announced** (`LOSS_UNKNOWN` + `prefix_missing`), which is the cost of (b). |
| 4 | Is `PAGE_READONLY` honoured for a BAR0 mapping? | `MmMapIoSpaceEx(PAGE_READONLY\|PAGE_NOCACHE)` fails, or reads through it fail | **Accepted and usable for reads [M]** (MAPPED, >20,000 reads). **Honoured, i.e. a store faults: [U]**; testing it means storing to the GPU's BAR0, which this lane forbids. |
| 5 | Is physical address == DMA address? | the address the driver programmed into the GSP (the mailbox value) does not lead to a valid LibOS array / queue region in CPU-physical RAM | **Yes on this VM [M]**: the mailbox value (a DMA address programmed into the GPU) read through `MmCopyMemory(physical)` gave a valid RMARGS array, a 129-page region whose every page-table entry lies in RAM and whose queue headers validated, 3 of 3 times (C, E gen 2, gen 3); the scan route validated the same structure independently (`method 2`). This is guest-physical == GPU DMA address in a KVM guest with a passed-through GPU; it says nothing about bare-metal Windows with VT-d DMA remapping [U]. |
| 6 | Are messages lost? Transport `seqNum` gaps? | decoder `seq_gap`/`parse_failure`, or an independent `seqNum` walk finding a jump | **No unannounced loss [M].** Independent walk (`seqs.py`) over all 7 attached queue instances of C-F (14 queue-direction streams): `seq_gaps 0` in every queue instance; decoder anomalies are only `prefix_missing` + `capture_loss` (all announced), `parse_failure` 0, `seq_gap` 0. `lost_messages_exact 0`, `torn_reads 0`, `invalid_elements 0`, `ring_refused 0`, ring high-water 814,208 B of 16 MiB. `LOSS_UNKNOWN` records: E 6 (gen 1 prefix 2, detach of gen 1 2, detach of gen 2 2), F 6 (gen 1 prefix 2, detach 2, gen 2 prefix 2), D 2 (prefix), C 0. The design doc's transport-`seqNum` falsifier (section 7) is **not** triggered. |
| 7 | Does the module disturb the box? | TDR (Display 4101 / nvlddmkm), Kernel-Power 41, bugcheck, driver hang, `nvidia-smi` failure while capturing | **No [M]**: 6 `pnputil /restart-device` of the display adapter (1.7 s each) under capture, `nvidia-smi` repeatedly, `dxdiag`; the System event log of the session (`cpu_and_events.txt`) holds no nvlddmkm/Display/WHEA/BugCheck/Kernel-Power-41 entries (only the shutdown/boot events of the single testsigning reboot). Cost: System-process CPU 0.0083 s/s idle baseline vs 0.0207 s/s with the collector attached (+1.2% of a core, one 40 s sample, H). |
| 8 | Control inventory vs the AD104 / Windows 580.88 inventory in the repo | n/a | see below |

### Control inventory (sizes and counts only, no values compared)

`inventory_comparison_summary.txt`, `inventory_comparison.tsv`. Reference: `kf-census --ogkm-tag 580.82.07` over the
five AD104 captures (`census/ref_all5/`): 221 distinct control ids (157 defined in some ogkm tag, 64 absent from
every tag). Decoded here with the same tag (the driver is 580.88 in both; tag per the task) and the same `--tier1` tables.

* One full boot (C, 150 s, no display modeset, no monitor): **206** distinct controls; union of C, D, E, F: **208**.
* Common with the reference: **183**; only here: **25**; only in the reference: **38** (18 defined, 20 absent-from-ogkm).
* The 64 "absent from every ogkm tag" (Windows-only) rows: **44 of 64 appear** on this box; 20 do not; 1 absent-from-ogkm id appears here that the reference never had.
* The 25 ids only here: 23 are not in the tier-1 tables at all (new to the repo; ids and sizes in the TSV), 1 defined, 1 absent: `0x00730404`, `0x20800156`, `0x208001a4`, `0x20801222`, `0x20801322`, `0x20801344`, `0x20801347`, `0x20801813`, `0x20801819`, `0x20801829`, `0x20801830`, `0x20803083`, `0x20803400`, `0x20803401`, `0x20803404`, `0x20808536`, `0x2080853b`, `0x20808542`, `0x20808546`, `0x20809038`, `0x2080a084`, `0x2080a0a4`, `0x2080a0f2`, `0x2080a63c`, `0x90e70113`.
* **Sizes: for all 183 common ids the set of request sizes and the set of reply sizes are identical** (0 differ) [M].
* Difference in conditions that [I] explains the 38 missing: this box had no monitor and no display session; the reference boots drove a display.

### Timeline of the mailbox on a driver restart (E gen 2, relative to the stream start, 10 ms polling)

`gson_E/cell_timeline.txt`; offsets there are relative to the first status-change event, about 25 ms earlier than BIND times in `census/E/binds_and_seqs.txt`.

## What failed or was awkward

* First two start attempts: harness quoting (`--layout=auto` is not an option; PowerShell script policy); no module defect.
* The first GSP-on capture run (C) used trace variant 1 only for the search phase; hence variant 2.
* Capture option (a) and (b) both found **nothing** in the default configuration, correctly: there is no queue.
* Not done: loading the module as a boot-start service so that it sees the very first GSP boot of a Windows start (the restart path was used instead; it exercises the same driver code, not the same boot path) [U]; a stored-fault test of `PAGE_READONLY` (forbidden); the scan worst case; HVCI/VBS on; a second die.

## Files

`default_A|A2|B/` sidecars (GSP off); `gson_C|D|E|F|H/` sidecars + mailbox timelines; `census/*/` decoder output; `census/ref_all5`
reference; `inventory_comparison*`; `preflight_default.txt`, `install_output.txt`, `cpu_and_events.txt`; `*.py`, `*.ps1` the
harness (small, read-only on the box except `gsp_on.ps1`, which sets the two registry values); `mbx_trace_v*.patch`.
