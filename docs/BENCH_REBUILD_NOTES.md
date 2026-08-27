# nvkvm Mode-2 bench rebuild status

---
## 2026-07-30 (task #104) — #13 root-cause candidate `enum_gr_sysmem(root_sys)`: ★ REFUTED. No patch landed.

**The hypothesis.** `nvkvm_m2_enum_gr_sysmem` (`nvkvm_gpu_emul.c:8836`) passes a hardcoded `false`
as the root aperture to `nvkvm_m2_pt_enum`, where its neighbour `nvkvm_m2_populate_cvas` (`:8876`,
fixed as M5.36) passes the *resolved* `root_sys`. Since `nvkvm_m2_pdb_is_compute` (`:8574`) returns
true unconditionally for UVM-managed roots, and UVM roots are the ones that *can* be sysmem-rooted,
a sys-rooted compute VAS would be mis-walked to **0 leaves** → nothing backed → host `Xid 31
FAULT_PDE`. Predicted signature in the existing `M6.5 enum_gr_sysmem:` log line: **`comp=1 runs=0`**.

**★ REFUTED on two independent grounds. The one-token change is a provable no-op on this workload.**

1. **No root is ever sysmem-rooted.** `grep -a "root=SYS"` returns **zero hits in every log ever
   captured on this bench** — 5 hanging runs, 1 passing run from task #95, and 9 fresh runs today.
   Every `M5.30 SET_PAGE_DIR UVM-VAS` line, in every run, is `aperture=0 root=FB`, for both UVM
   roots (`hVASpace=0xcaf00005 PDB=0x3400000`, `hVASpace=0xcaf00062 PDB=0x3401000`). So
   `chan_vas[v].root_sys == false` for all of them and the hardcoded `false` is already the
   resolved value. Note this contradicts the comment at `:2740` ("the UVM root is *typically* in
   SYSMEM") — on GA106 + 580.159.04 open, measured, it never is.
2. **`comp=1 runs=0` is not a failure signature.** It is a near-constant of both outcomes:

   | run | verdict | sweeps | `comp=1` | `comp=1 runs=0` |
   |---|---|---|---|---|
   | `c8iter_r2` | **PASS** | 165 | 99 | **40** |
   | `c8iter`, `c8iter_r3` (`fc4164d`) | HANG | 160 | 96 | **38** |
   | `ab862_1/2/3` (`862c7c2`) | HANG | 160 | 96 | **38** |
   | 9 fresh runs today | PASS | 165 | 99 | **40** |

   The **passing** run has *more* `comp=1 runs=0` than the hangs. The 5-sweep delta is just the
   pass completing two more iterations. Steady `runs=0` producers are `vas=0x5c000008
   pdb=0x3110000` (33/33 sweeps, both outcomes) plus a few early sweeps of the two UVM roots
   before their tables are populated — all FB-rooted, all benign.

**Four-way verdict on the faulting address: NOT IN OUR TABLE (capture gap) — in 5/5 hangs.**
Mapping each hang to its host `Xid` and searching its QEMU log for the faulting VA at exact,
2 MiB and 512 MiB granularity: **zero hits, in all five.** The instrument is not blind — the same
logs print `M6.5 back_sys VA=0x…` / `M5.7 back_and_map[…] VA=0x…` for VAs in the same range and
format. So this is consistent with the hypothesis's *conclusion* (we never recorded the mapping)
but not with its *mechanism*, which is inert.

**★ What the data says instead — a second, never-enumerated VA region 260 MiB below the working set.**
The geometry is startlingly regular across all five hangs. Backed 0x7-half span is always
`0x…2400000 .. 0x…3b33000` (23.2 MiB); every fault lands **below** it, never inside:

| run | faulting VA | engine | backed span base | `base − 0x10400000` | offset into it |
|---|---|---|---|---|---|
| `c8iter` | `0x77ad7f000000` | CE2 HUBCLIENT_CE0 | `0x77ad80400000` | `0x77ad70000000` | `+0xf000000` |
| `c8iter_r3` | `0x7024d200a000` | GRAPHICS GPC2 | `0x7024e2400000` | `0x7024d2000000` | `+0xa000` |
| `ab862_1` | `0x7984f4009000` | GRAPHICS GPC1 | `0x798502400000` | `0x7984f2000000` | `+0x2009000` |
| `ab862_2` | `0x7b9072009000` | GRAPHICS GPC1 | `0x7b9082400000` | `0x7b9072000000` | `+0x9000` |
| `ab862_3` | `0x785ab2009000` | GRAPHICS GPC1 | `0x785ac2400000` | `0x785ab2000000` | `+0x9000` |

There is a 256 MiB-aligned region at exactly `span_base − 0x10400000` that we back **nothing** in,
and three of the four GRAPHICS faults are a bare `+0x9000`/`+0xa000` into it. The passing run backs
the *same* single 23.2 MiB region and nothing below — i.e. the pass is not "we captured more", it is
"the guest never reached down there". **Next probe should identify what CUDA puts 260 MiB below the
cuMemAlloc working set and why only some runs touch it** (kernel local-memory/stack backing store
grown at the first N=2048 launch is the obvious candidate, and would explain ITER 3).

**No error path is involved.** In every hang: `back_sys` failures 0, budget truncation 0,
`STALE-SYS` 0, and the #13 CE-PT-write trigger `#13 PT-SYNC@release` fires 38–40× with
`backed>0` every time (47× in the pass — it runs longer). Everything the emulator attempts,
it completes. The bug is purely that an access arrives for a VA it was never told about.

**★★ #13 DID NOT REPRODUCE AT ALL TODAY — 9/9 PASS — while #14 still reproduces on demand.**
This is why no patch was landed even speculatively: there is currently no failing measurement to
fix against.

| revision | emulator md5 | binary md5 | runs | result |
|---|---|---|---|---|
| `c9cfe01` (HEAD) | `0e2ac537ebb0f68bad59514f69037ac7` | `cd61bc8c5d0c9c7cb15847387ff7f9c1` | 5 × `ITERS=5` | **5 PASS / 0 HANG** |
| `fc4164d` | `cced661c16f6856801d16dae151bc2f0` | `d7dd2573b87b9c1a9ccc6bb73d9a96dd` | 3 × `ITERS=5` | **3 PASS / 0 HANG** |
| `fc4164d` | ″ | ″ | 1 × `ITERS=16` | **PASS** (all 16, incl. three N=2048) |

The `fc4164d` binary was rebuilt to `d7dd2573b87b9c1a9ccc6bb73d9a96dd` — **bit-for-bit the binary
task #95 measured 1 PASS / 2 HANG on.** Same host boot session (up since Jul 28), same host driver
load (Jul 29 11:03), same guest kernel 6.8.0-117, same harness. At #95's ~25 % pass rate, 9/9 green
has probability ~4e-6, so the rate genuinely changed; the variable is environmental and not yet
identified. `c9cfe01` vs `fc4164d` is *not* it — their only code delta is inside
`if (nvkvm_rec_on() && …)` and the recorder is off (`NVKVM_M2REC` unset ⇒ no `m2rec` property).

**★ Instrument check, and the reason the above is trustworthy: `mp14_run_guest.sh MP14_N=2` still
reproduces #14 exactly as #95 recorded it** — `pass=1 fail=1`, loser stalled at `cuCtxCreate`,
4 host Xids. Bench, harness, GPU and binary are all healthy; #13's non-reproduction is a real
observation about #13, not a broken rig.

**Evidence preserved.** #95's raw logs were still in the bench's `/tmp` and are the only extant
recording of a #13 hang; copied to **`vh:/root/hang_evidence_20260729/`** (6.7 MB, 15 files:
`{c8iter,c8iter_r2,c8iter_r3,ab862_1..3}_{delta.txt,guest.log}`, both emulator sources,
`host_xids_jul29.txt`). `/tmp` on this bench is not durable — do not leave the next hang there.

**Traps worth not rediscovering:**
- `bench_boot.sh` does `rm -f $OVL`, so the guest overlay is wiped every boot: **re-`scp` the test
  `.c` *and* the runner script on every cycle**, not just the first.
- Mapping a hang to its Xid is by wall-clock: the `*_guest.log` mtime is written at the *end* of the
  run, ~2 min *after* the Xid. Do not assume the nearest-later Xid.
- Two Xid classes are live here and must not be conflated: `#13` = `GRAPHICS GPC*
  GPCCLIENT_T1_2` / `CE2 HUBCLIENT_CE0`, `FAULT_PDE VIRT_**WRITE**`, VA `0x7xxx_xxxxxxxx`
  (a guest UVM device VA); `#14`/concurrent = `*_PBDMA* HUBCLIENT_ESC`, `VIRT_**READ**`,
  VA `0x2_00xxxxxx`. Grepping "Xid" alone mixes them.
- Bench state left as found: emulator source `0e2ac537ebb0f68bad59514f69037ac7` (= `c9cfe01`'s
  `nvkvm_gpu_emul.c`, unchanged by this task) deployed and installed as binary
  `cd61bc8c5d0c9c7cb15847387ff7f9c1`; `.srcrev` = `c9cfe01`; no QEMU running. **This task landed
  no source change**, so the md5s above remain valid for any later commit that does not touch
  `src/qemu/`. Trust the md5, not the commit id.

---
## 2026-07-29 (task #96) — `cap1b`: GSP-D6 made observable, reply stream proven unchanged

**SOURCE REVISION.** Emulator `819282d` (`nvkvm_gpu_emul.c` md5 `2132bbdbf98ab85449e9513c9c230bbf`),
built and installed here: `/opt/qemu-nvkvm/bin/qemu-system-x86_64` md5
`b21892d86716574acf29828663b31c68`. The three control/after captures in this section all ran on
binaries whose source md5 was verified byte-identical to the local commit. Before the change the
bench served `cced661c16f6856801d16dae151bc2f0` = `dc7aaaf`/`264caa2` (binary
`d7dd2573b87b9c1a9ccc6bb73d9a96dd`).

**What changed.** `nvkvm_m3_service_cmdq` now READS the continuation elements of a multi-element
GSP message and discards them, so the recorder witnesses them. Gated on `nvkvm_rec_on()` — a
non-capture run is bit-identical. GSP-D6 (act on element 0 only) is untouched and still a
divergence; only the *observability* changed. See `traces/mode2_c_reference/README.md`.

**The proof, and the reason it needs three captures.** Two captures of the SAME binary are not
identical here — measured, not assumed. Noise floor (`before_A` vs `before_B`, both `dc7aaaf`)
versus the after-run (`before_B` vs `cap1b`):

| projection | control | after |
|---|---|---|
| `MmioWrite` | 216 520 / 216 520, 460 differ | 216 520 / 216 520, **460 differ** |
| `GuestWrite` CONTENT | 859 / 859, 2 differ | 859 / 859, **2 differ** |
| `IrqRaise` | identical | identical |
| `GuestRead` | 629 / 629 | 629 / **661** |

★ The 460 differing `MmioWrite` positions are the **identical index set** across all three
pairings (zero symmetric difference) — they are writes carrying guest physical addresses, which
move every boot. The 2 differing replies are the same two elements every time (`seq=3 fn=228`,
`seq=165 fn=76`), both wall-clock-bearing. Instrument: `scripts/mode2_diag/rec_replydiff.py`.

**The non-capture path is unaffected, positively.** One fresh boot at `819282d` with the recorder
OFF and full forwarding (`bench_boot.sh`, `m2cefwd=on`): `cup2` **rc=0**, `CE rv=0xabcd1234` byte
exact, `cuCtxCreate` OK, `cuDeviceTotalMem` 11909 MiB. The continuation read is inside
`if (nvkvm_rec_on() && ...)`, so this is what "changes only what we witness" means in practice.

**Traps this cost, worth not rediscovering:**
- ★ **A single-stream positional diff of two traces is useless.** The guest's PTIMER/mailbox poll
  loop desynchronises around record ~139 900 and then ~220 000 of 360 000 records "differ". That
  is the projection failing, not the device. Diff each KIND as its own subsequence — those are
  stable across boots even though their interleaving is not.
- ★ **Guest physical addresses move every boot.** The GSP message queues landed at `0x128c41000`,
  `0x124641000`, `0x127601000` in three consecutive runs. Any GPA-absolute comparison is noise.
- ★ **`nvidia-smi -q | head -40` SIGPIPEs `nvidia-smi` mid-enumeration** and silently truncates
  the RPC stream — and `$?` after the pipeline is `head`'s, so it reports `rc=0`. The original
  `cap1` has 563 `GuestWrite`s; the untruncated run has 859. Never pipe the workload.
- `emulator-src-commit` was **blank** in all four original capture headers: the bench tree is not
  a git checkout, so `git rev-parse` yielded nothing, silently. `run_mode2_vm.sh` now falls back
  to `/workspace/nvkvm/.srcrev` then `$NVKVM_SRCREV` and prints `UNKNOWN` rather than a blank.
  **Update `.srcrev` whenever you sync `src/qemu/` to the bench.**
- Deploying just this file is a plain `cp /workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c
  /opt/qemu-src/hw/misc/` — it has no `../../src/common/*.h` includes, so it needs none of the
  `nvkvm_inc/` rewriting the other nvkvm sources do. `ninja && ninja install` in `/opt/qemu-src/build`.

---
## 2026-07-29 (task #95) — #14 validated on HW at `fc4164d`; ★ the "cup8_iter 5/5 green" line below is WRONG

First hardware run of the post-`862c7c2` emulator (`#14 P0`/`P1` had never executed on a GPU —
see the `-Werror=redundant-decls` section below).  **Every result here carries its revision**,
because the whole reason this task existed is that the bench silently served a stale binary.

**SOURCE REVISION — how it was established, do this every time.**  Local `consolidation` HEAD
`fc4164d`; bench `/workspace/nvkvm` and the deployed `/opt/qemu-src/hw/misc/` were verified
byte-identical to it (`md5sum` over every `src/{qemu,common,abi}` file — only `nvkvm_handle.c`,
`nvkvm_isolate.c`, `virtio_nvgpu.h` differ, and only by the `nvkvm_inc/` include rewrite).
`touch nvkvm_gpu_emul.c && ninja` reproduced the installed binary **bit-for-bit**
(`qemu-system-x86_64` md5 `d7dd2573b87b9c1a9ccc6bb73d9a96dd`, emulator source md5
`cced661c16f6856801d16dae151bc2f0`) — which is what proves the running binary is that source.
`NVKVM_STUB_EMBEDDED` is *not* defined in this build, so the isolate uses the on-disk fallback
`/usr/lib/nvkvm/nvkvm_stub`; keep it rebuilt or forwarding silently degrades (commit `4f52877`).

**★★ CORRECTION: `cup8_iter` (#13) is NOT reliably green, at ANY revision.**  The
"LADDER RESULT 2026-07-29 — ALL GREEN … cup8_iter rc=0 … (#13 stays fixed)" line further down
this file was a **single lucky sample** and does not reproduce.  Measured, one fresh boot each:

| revision | cup8_iter (ITERS=5) | result |
|---|---|---|
| `fc4164d` (HEAD)  | 3 runs | **1 PASS / 2 HANG** |
| `862c7c2` (old baseline, A/B rebuild) | 3 runs | **0 PASS / 3 HANG** |

The hang is always the same: `ITER 0/1/2` (N=512/1024/1536) pass byte-exact, then **ITER 3
(N=2048) never completes**; the process sits `State=R` (busy-poll, not a D-state wedge) and the
**host** logs `Xid 31 … MMU Fault … FAULT_PDE ACCESS_TYPE_VIRT_WRITE` against the isolate
(engine varies: `CE2 HUBCLIENT_CE0`, `GRAPHICS GPC1/GPC2 GPCCLIENT_T1_2`).
⇒ **`fc4164d` is NOT a regression on `862c7c2`** — it is marginally better.  #13's memory entry
("RESOLVED") and this file's green ladder both overstate it: at 2048² inside a multi-iteration
process the fix holds only ~1 run in 4.  Standalone `cup8` at N=2048 passes byte-exact both
before and after five host Xids, so the GPU is not degraded and the effect is real.

**Baseline at `fc4164d`, one fresh boot each** — `cup2` rc=0 (CE `rv=0xabcd1234` byte-exact),
`cupctx2_min` rc=0 (#12 stays fixed), `cup8` 2048² `bad=0 maxerr=0` (host GPU util 10%),
`cup8_iter` as above.

**★ #14 — TWO CONCURRENT CUDA APPS: REPRODUCES at `fc4164d`, 2/2 runs, deterministic.**
Exactly one process finishes 2048² byte-exact; the other **hangs in `cuCtxCreate`** — which is
precisely the behaviour commit `65281f2` documents ("Baseline = P1 … 2× concurrent = winner
reliably passes").  Only `#14 P0`+`P1` ever landed; P2/P3 are BANKED, P4 was reverted, there is
no P5/P6.  **#14 is an open, explicitly-deferred problem, not a completed refactor.**

Where it stops, measured (`scripts/mode2_diag/mp14_run_guest.sh`, added by this task):
- loser: `State=R`, `wchan=0`, **empty kernel stack, not in a syscall** ⇒ spinning in libcuda
  userspace, *not* stuck in an RM ioctl.  Guest dmesg clean, no guest Xid, no host Xid.
- emulator: the loser's guest RM spins forever on GSP RPC `fn=76 ctrl=0x20801702`
  (`NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`) — the completion poll.  The spin starts the instant
  the winner's client is root-freed (`#14 P1 PROC[n] reaped`), i.e. when the last doorbell any
  process will ever ring has been rung.  Exactly the `r8b` starvation banked in `65281f2`.
- the `m2_poll_kick` (piece-2, `nvkvm_gpu_emul.c:3003`/`:3534`) *does* fire and re-rings the
  last doorbell token, and it does not help — re-running the service re-walks a channel the
  loser's VAS still cannot resolve.
- **identical-VA collision, confirmed in the address table.**  Neither user process's own PDB
  ever resolves the channel VA: `pdb=0x3401000` 0 hits / 307 walks, `pdb=0x3405000` 0/268
  (run 2; 0/273 and 0/308 in run 1).  *All* successful resolution goes through the shared VASes
  `pdb=0x2efa6c000` (FB, 319) and `pdb=0x3118000` (SYS, 36).  Both processes' compute channels
  carry the same client and execute with `chan_pdb=own_pdb=0x3118000`; the GPFIFO VA
  `0x120064000` is used by both.

**★ NEW latent defect in P1, found only because P0/P1 finally ran on hardware.**
`nvkvm_m2_proc_add_client()` (`nvkvm_gpu_emul.c:5140`) has **no cross-proc uniqueness check**,
and the comment at `:2767` asserts "kernel-internal clients … only ever appear on the DST side"
— which is true, but the code then assumes each process has *its own* dup-DST client.  Hardware
says otherwise: **both processes' dup edges share ONE dst client** (`0xc1d00001`), so it is
registered into both procs:

    #14 P1 PROC[0] += client=0xc1d00001 (clients_n=2)
    #14 P1 PROC[1] += client=0xc1d00001 (clients_n=2)

`nvkvm_m2_proc_find_by_client()` returns the FIRST match ⇒ every lookup through that client
resolves to `PROC[0]`, and `nvkvm_m2_proc_drop_client()` unlinks it from `PROC[0]` only, leaving
`PROC[1]` holding a freed client.  Its one consumer today is the M5.11 doorbell-demux log
(`:3803`), which keys on `s->chans[].client` — exactly the shared client — so **P1's
"distinct vChid→chan→proc across 2× cup8" acceptance signal was measuring an aliased mapping.**
NOT fixed here: #14 is deferred to the Rust rewrite and patching a banked C feature would be a
redesign.  **The design lesson is load-bearing for the rewrite's per-Proc ExecPlane:** a `Proc`
cannot be modelled as *a set of RM clients* — the guest's single `nvidia-uvm` gpu-ops client is
global.  Key on the anchor (dup-SRC) client only, or on `(client, PDB)` / `(client, vChid)`.

Harness added: `scripts/mode2_diag/mp14_run_guest.sh` — N concurrent `cup8`s on one fresh GSP;
unlike `cup8_concurrent_run_guest.sh` it reports *where* a hang is (last CUDA API line,
`/proc/PID/syscall` decoded to `ioctl(fd)` → device node, `wchan`, kernel stack, R-vs-D).

---
## 2026-07-29 — "Mode-2 guest boot failure" on the freshly rebuilt bench: ★ THERE WAS NO BOOT FAILURE

Reported symptom: "QEMU exits cleanly during guest boot, /tmp/m0_serial.log stops at ~4.1 s,
the guest never reaches userspace and never opens SSH on 2223." Every part of that turned out
to be an artefact of how the bench was being *observed*. Ladder is green (see bottom).

**What was actually true.** The QEMU from the "failed" boot was still running 2 h 34 min later.
Its serial log ran all the way to `cloud-final.service` / `cloud-init.target` / a
`nvkvm-guest login:` prompt at t=21.9 s. The forwarding path was up on every boot (M5.1 isolate
ready, M6.1 SHARED, MEMTEST PASS, M5.3 rc=0). Kernel was the pinned 6.8.0-117-generic and
/home/ubuntu/nvmods held all four .ko.

**Root cause of "never opens SSH": a missing ssh identity on the BENCH HOST, not the guest.**
`/root/.ssh` on the bench had `guest_key`/`guest_key.pub` (the keypair baked into seed.iso's
`ssh_authorized_keys`) but **no default identity** — no `id_ed25519`, no `id_rsa`, no
`~/.ssh/config`. Every one of the ~30 harness scripts under `scripts/mode2_diag/*_host.sh`
runs a **bare** `ssh -p 2223 ubuntu@localhost` with no `-i`, so ssh offered no key at all and
the guest answered `Permission denied (publickey,password)`. Read as "the guest never came up".
Proof: with `-i /root/.ssh/guest_key` the exact same live guest logged in immediately.

FIX (bench host, one time per rebuild) — `/root/.ssh/config`:

    Host guest vg
        HostName 127.0.0.1
        Port 2223
        User ubuntu
        IdentityFile /root/.ssh/guest_key
        StrictHostKeyChecking no
        UserKnownHostsFile /dev/null
        LogLevel ERROR

    Host localhost 127.0.0.1
        IdentityFile /root/.ssh/guest_key
        StrictHostKeyChecking no
        UserKnownHostsFile /dev/null
        LogLevel ERROR

The second block is the load-bearing one: it makes every existing bare
`ssh -p 2223 ubuntu@localhost` / `scp -P 2223` in the repo work unmodified. **Add this to the
guest-disk phase of any future rebuild** — it is as much a part of "the bench works" as the
qcow2 is. `chmod 600 /root/.ssh/config`.

**Root cause of "stops at 4.1 s": looking too early.** This guest needs ~20-25 s to reach a
login prompt, and QEMU's `-serial file:` output is buffered/lazy, so a peek a few seconds in
shows the log frozen mid kernel-init (`clk: Disabling unused clocks`, `RAS: Correctable Errors
collector initialized`, ...) with no further growth. Measured on a fresh overlay this run: the
log sat at t=7.9 s and did not move for ~30 s, then jumped straight to a login prompt at
t=20.9 s. Do not conclude "hang" from a log that has merely not caught up.

**★ TRAP: the documented verify step is a guaranteed false negative.**
`pgrep -x qemu-system-x86_64` and `pgrep -a qemu-system-x86_64` match `/proc/PID/comm`, which
the kernel truncates to 15 chars — `qemu-system-x86`. The `_64` form can therefore **never**
match. Measured with a live QEMU (pid 476684, up 2 h 34 m):

    $ pgrep -a qemu-system-x86_64   -> (nothing)
    $ pgrep -x qemu-system-x86_64   -> rc=1
    $ pgrep -x qemu-system-x86      -> 476684

So "verify pgrep is empty before launching" always passes, you launch a second QEMU, it loses
the race for the hostfwd port and dies with `Could not set up host forwarding rule
'tcp::2223-:22'`. **Verify with `pgrep -x qemu-system-x86`** (and independently with
`ss -tln | grep 2223`, which does not lie). The `[4]` bracket form is still required for
`pkill -f` so it does not match its own command line.

**Not confirmed: the SIGHUP theory.** Launching `bash /root/boot_mode2.sh >log 2>&1 &` from an
ssh command that then returns did NOT kill QEMU — it survived the session teardown and booted
to a login prompt. `run_mode2_vm.sh` `exec`s QEMU with `-display none` and no controlling tty.
Detached launch is still the right habit, but a non-detached launch is not what was breaking
this bench.

**Also ruled out** (all checked, all clean): no assert/segfault/abort/OOM; QEMU exit status
never observed because QEMU never exited; PCI enumeration and MSI-X fine (the blacklist is
baked in and the emulated GA106 at 00:07.0 is pristine); 9p mounts fine; `-no-reboot`
semantics irrelevant (not passed); host driver healthy 580.159.04 throughout, 0 Xid.

**Cosmetic noise worth knowing:** with `-d unimp,guest_errors` the q35 machine's unused
ich9-ahci emits a steady `ahci: IRQ#2 level:1` into /tmp/m0_qemu.log — 13929 of 13938 lines in
one 2.5 h run. Harmless, but it buries the nvkvm markers; always
`grep -a 'nvkvm-gpu\|M5\.\|MEMTEST'` rather than `tail` the QEMU log.

**Added to the repo this session:** `scripts/mode2_diag/bench_boot.sh` (kill -> real verified
wait on the truncated comm AND on port 2223 -> fresh overlay -> detached launch) and
`scripts/mode2_diag/bench_wait.sh` (block for ssh, then print the M5.1/M6.1/MEMTEST/M5.3
markers, and tell you to suspect ssh AUTH if the serial log ends at a login prompt). Use these
two as SEPARATE commands instead of hand-rolling the restart.

LADDER RESULT 2026-07-29 — ALL GREEN, one fresh boot each, host 580.159.04 open, guest STOCK
(unpatched; `mode2_uvm_complete_proof.patch` was NOT needed):
- cup2 rc=0 — cuInit / RTX 3060 compute 8.6 11909 MiB / cuCtxCreate / cuMemAlloc /
  CE HtoD+DtoH `rv=0xabcd1234` byte-exact PASS
- cupctx2_min rc=0 — CTX1 create+destroy OK, CTX2 create+destroy OK, VERDICT PASS (#12 stays fixed)
- cup8 rc=0 — 2048² matmul `bad=0 maxerr=0 C[0]=2048` VERDICT PASS
- cup8_iter rc=0 — ITER0 512 / ITER1 1024 / ITER2 1536 / ITER3 2048 / ITER4 768, all
  `bad=0 maxerr=0`, VERDICT PASS (iters=5 fails=0) (#13 stays fixed)
  ★★ **THIS LINE IS A SINGLE LUCKY SAMPLE — see the task-#95 section at the top of this file.**
  Re-measured the same day: 0 PASS / 3 runs at this very revision (`862c7c2`), 1 PASS / 3 at
  `fc4164d`. cup8_iter hangs at ITER3 (N=2048) with a host `Xid 31 … FAULT_PDE`. Do not cite
  it as evidence that #13 is fixed.
Bench is unblocked for #90 (C reference traces) and #47.

---
## 2026-07-29 (later) — #90 capture campaign: ★ the bench had never compiled anything past 862c7c2

Recording the four §6 reference traces (`traces/mode2_c_reference/`) turned up a build fact
nobody had written down.

**★ Every emulator revision from `3710b8e` onward fails to build here, and always has.**
`nvkvm_gpu_emul.c` gained a **duplicate forward declaration** of `nvkvm_m2_is_gr_client` /
`nvkvm_m2_is_user_client` in `3710b8e` (#14 P0). The bench's QEMU 9.2 configure carries
`-Werror=redundant-decls`, so:

    error: redundant redeclaration of 'nvkvm_m2_is_gr_client' [-Werror=redundant-decls]

⇒ the green ladder recorded above ran at **862c7c2**, and `#14 P0/P1` (`3710b8e`, `9ff481b`,
`65281f2`) had **never run on hardware**. Fixed in the #90 commit; both were then re-validated
in the act of capturing (`cup2` rc=0, `cup8` 2048² `bad=0 maxerr=0`, recorder on).

**Trap: syncing `src/qemu/` wholesale into `/opt/qemu-src/hw/misc/` breaks the build.**
The provisioned tree has its `#include "../../src/common/*.h"` paths **rewritten** to
`nvkvm_inc/*.h`, and it has more of them rewritten than `build_qemu.sh` step 5 documents
(`nvkvm_isolate_proto.h`, `nvkvm_abi.h`, `nvkvm_ring.h`, plus `nvkvm_handle.c` and
`nvkvm_isolate.c`, not only `virtio_nvgpu.h`). Copy only the files you changed, or re-run the
rewrite over every `*.c`/`*.h` afterwards.

**Trap: `meson.build` here is hand-extended**, listing ~10 nvkvm sources in one block rather
than the two blocks `build_qemu.sh` generates. Its "already patched?" guard is on
`virtio_nvgpu.c`, so a NEW source file is never added by re-running the script. `build_qemu.sh`
now has a separate idempotent step that anchors on the `'nvkvm_gpu_emul.c'` line wherever it
sits.

**Capturing a trace** — `scripts/mode2_diag/rec_capture.sh` (host side) does the safe restart
with full `m2*` control (`bench_boot.sh` forces `m2cefwd=on`, which is wrong for a hermetic
capture). The trace file is only complete after QEMU **exits**; a live file is a usable dense
prefix. `sudo poweroff` in the guest is the clean way to end a capture. Decode with
`scripts/mode2_diag/rec_dump.py`.

**Also measured**, worth knowing before someone debugs it as a regression: with a fresh overlay
the guest reaches ssh in **10-14 s** here, not 20-25 s, and `nvidia-smi -q` enumerates the
emulated GA106 **with `m2fwd=off`** — the fake-boot path alone gets all the way to
`GSP_INIT_DONE`.

---
## REBUILD 2026-07-19 (box #45305458 @ 70.30.158.46:27130) — ✅ COMPLETE (all baselines green)
Fresh BLANK vast box (RTX 3060 GA106, host 575.51.03, kernel 6.8.0-59, /dev/kvm present,
21 cores / 49GB / 138G free). Goal: single-process baseline GREEN at emulator source = 862c7c2
(local HEAD c861451 = 862c7c2 + 2 docs-only commits; nvkvm_gpu_emul.c byte-identical to 862c7c2).
NOTE: /workspace/bench-archive did NOT survive on this box — VBIOS rsync'd from local
(/workspace/bench-archive/ga106_vbios.rom md5 48df40a04432aca6a35bee2785857eba).

Phase status (this rebuild):
- [x] 0. Repo rsync'd to /workspace/nvkvm; VBIOS -> /opt/nvkvm-guest/ga106_vbios.rom (md5 OK)
- [x] 1. Host apt deps DONE (added qemu-utils cloud-image-utils genisoimage swtpm; box is Ubuntu 22.04/jammy host)
- [x] 2. DRIVER DECISION = **575-host + 580-source** (task fallback path).
      The 580.159.04 .run REFUSED to install (`--silent`) because the vast box ships the 575
      driver via apt/dpkg ("installed through packages") and the .run cancels on an alternate
      installation; no override flag exists in `--help`. Purging the apt 575 set on a rented GPU
      box risks bricking host GPU access, so per the runbook fallback I kept 575 RUNNING on the
      host and staged the 580 open SOURCE for the guest build (ABI auto-detect handles 575-host
      per multi_driver_validated: "575.51.03 -> profile 570: matmul PASS"). Reloaded 575
      (nvidia-smi = 575.51.03, healthy) after the failed install, then `--extract-only`'d 580.
      Staged from the extracted tree:
        - /usr/src/nvidia-580.159.04/  = kernel-open source (nv-kernel.o_binary present) [ogkm 9p]
        - /usr/lib/firmware/nvidia/580.159.04/{gsp_ga10x,gsp_tu10x}.bin              [nvfw 9p]
        - /workspace/nvkvm/host-libs-580/{libcuda,ptxjitcompiler,allocator,nvvm}.so.580.159.04
          + cuda.h (cudart 12.6 redist, CUDA_VERSION 12060)  [pulled into guest via nvkvm_src 9p]
- [x] 3. Build QEMU DONE — /opt/qemu-nvkvm/bin/qemu-system-x86_64 lists m2fwd/m2exec/m2cefwd.
      TWO MORE build_qemu.sh bugs found+fixed (committed):
      (a) step-5 sed used '|' as BOTH s-delimiter and regex-alternation (common|abi) -> "unknown
          option to s". Fixed: delimiter -> '#'.
      (b) step-6b virtio.c patch regex required a trailing comma after "virtio-gpio", but in
          QEMU 9.2.0 [VIRTIO_ID_GPIO] is the LAST initializer entry with NO trailing comma.
          Fixed: made comma optional in match, emit our entries with the comma.
- [x] 4. Build stub DONE — make -C src/stub nvkvm_stub (156656 B) -> install /usr/lib/nvkvm/nvkvm_stub.
- [~] 5. Guest disk IN PROGRESS:
      - base = ubuntu 24.04.4 noble cloudimg -> /opt/nvkvm-guest/ubuntu-24.04.qcow2 (+30G, 33.5G virt)
      - seed.iso built with cloud-localds; user-data (plain #cloud-config, NOT base64 this time —
        no passwd-escaping issue since keys are ssh-ed25519 one-liners) has BOTH pubkeys
        (local root@test-real-amd64-host for `ssh vg`; vh root@ubuntu for harness `ssh -p 2223`),
        ssh_pwauth:true + ubuntu:nvkvm fallback, and BAKES /etc/modprobe.d/nvkvm-blacklist.conf.
      - provisioning boot: used the nvkvm qemu (no distro qemu on box) as a PLAIN boot on port 2222
        (no -device nvkvm-gpu-emul), shares nvkvm_src + ogkm 9p. Launch via
        /opt/nvkvm-guest/boot_provision.sh with `setsid ... </dev/null & disown` + wait loop in ONE
        ssh session (short-nohup gets orphaned — ssh_aliases gotcha).
      - kernel 6.8.0-117: apt-installed image+headers+modules(+extra)+build-essential; GRUB_DEFAULT
        pinned to the 117 advanced menuentry id; apt-mark hold on the 4 kernel pkgs; unattended-
        upgrades removed. Rebooted -> `uname -r` = 6.8.0-117-generic CONFIRMED.
      - 580 open modules: mounted ogkm 9p, cp -a to /root/nv580src, `make -j4 modules
        SYSSRC=/lib/modules/6.8.0-117/build` (4G swap added). NVBUILD_RC=0.
        vermagic=6.8.0-117-generic, version 580.159.04. Staged 4 .ko ->
        /home/ubuntu/nvmods/{nvidia,nvidia-uvm,nvidia-modeset,nvidia-drm}.ko (ubuntu:ubuntu).
      - guest userspace staged: /usr/local/nvidia-guest/lib/{libcuda,ptxjitcompiler,allocator,
        nvvm}.so.580.159.04 (+ .so.1/.so symlinks via ldconfig, ld.so.conf.d entry) + cuda.h
        -> /usr/include/cuda.h (CUDA_VERSION 12060).
      - Clean `poweroff` to persist base qcow2. run_mode2_vm.sh boots this base with a persistent
        mode2-overlay.qcow2 + shares ogkm/nvfw/nvkvm_src.
   [x] 5. Guest disk DONE.
- [~] 6. Mode-2 smoke cup2 — FIRST ATTEMPT FAILED on host=575, DRIVING A HOST-DRIVER SWAP TO 580.
      cup2 boots the Mode-2 VM fine: emulated GA106 at 00:07.0, host isolate spawned, MEMTEST
      PASS, OS_DESCRIPTOR guest-RAM pin rc=0. cuInit OK, RTX 3060 detected (compute 8.6, 11909MiB),
      cuDeviceTotalMem OK — then HANGS at cuCtxCreate's CE path -> **rc=124 (timeout), DETERMINISTIC
      across 2 fresh boots**. QEMU log = wall of `DIAG vas[N] hvas=.. pdb=.. eva=.. -> FAULT`
      (260-600 faults) during the CE MEMSET/COPY setup = the address-table VA->phys resolution
      MISSES. cup2 busy-polls (State=Rl, 100% CPU, NOT D-state; guest dmesg CLEAN, no Xid; host GPU
      healthy). => This is the host-driver-version dependency: the known-good baseline (862c7c2) was
      validated with **host driver 580.159.04**; on host 575 the emulator's CE VAS resolution faults
      and cuCtxCreate hangs. DECISION REVISED: must install 580 on the host after all. The .run
      refused earlier due to the apt-managed 575 driver -> now purging the apt 575 set + installing
      the 580 .run (mechanics per multi_driver_validated).
      DID IT: rmmod nvidia*, `apt-get purge` the whole nvidia-driver-575/dkms/utils/libnvidia-*575
      set (kept container-toolkit + nvidia-modprobe, harmless), then
      `sh NVIDIA-580.run --silent --no-x-check --no-nouveau-check --dkms -m=kernel-open` -> RC=0.
      **HOST NOW ON 580.159.04** (nvidia-smi healthy, open modules loaded, RTX 3060 responsive).
      /usr/src/nvidia-580.159.04 + /usr/lib/firmware/nvidia/580.159.04 (guest 9p shares) intact.
      >>> DRIVER DECISION FINAL = **580.159.04 INSTALLED ON HOST** (matches known-good baseline).
   [x] 6. Mode-2 smoke cup2 rc=0 ON HOST=580 — cuInit OK, RTX 3060 (8.6, 11909MiB), cuCtxCreate OK,
      cuMemAlloc OK, CE HtoD/DtoH byte-exact (rv=0xabcd1234 -> PASS). CONFIRMS the host-driver-version
      dependency: 575 hung at cuCtxCreate CE (VAS faults); 580 passes. (Early transient VAS faults are
      normal — the address table is forward-populated, miss-before-populate = fault by design.)
      GUEST GOTCHA: each fresh overlay lacks /usr/lib/x86_64-linux-gnu/libcuda.so (unversioned, needed
      by `gcc -lcuda`); re-add `ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04
      /usr/lib/x86_64-linux-gnu/libcuda.so; ldconfig` after each fresh boot (or bake into base).
- [~] 7. Baseline (each FRESH boot, host=580):
      [x] cupctx2_min (#12) rc=0 — CTX1 create+destroy OK, CTX2 create+destroy OK, VERDICT PASS
          (2 contexts). #12 fix (f5bb32f in 862c7c2) confirmed. 0 faults this boot.
      [x] cup8 rc=0 — 2048^2 matmul byte-exact (bad=0 maxerr=0) VERDICT PASS (host GR matmul at scale)
      [x] cup8_iter (#13) rc=0 — ITER0 N=512 PASS, ITER1 N=1024 PASS, ITER2 N=1536 PASS (the old
          #13 hang point), ITER3 N=2048 PASS, ITER4 N=768 PASS. VERDICT PASS (iters=5 fails=0).
   [x] 7. Baseline DONE — ALL THREE GREEN on host=580. Host GPU healthy (0% util, 1 MiB), no Xid.

=== REBUILD COMPLETE 2026-07-19 — ALL SUCCESS CRITERIA MET ===
cup2 rc=0 (smoke) | cupctx2_min rc=0 (#12, 2 ctx) | cup8 rc=0 (2048^2 byte-exact) |
cup8_iter rc=0 (#13, 5/5 iters). Driver decision FINAL = 580.159.04 installed on the host
(575 host deterministically hangs cuCtxCreate's CE VAS resolution; 580 passes — host-driver-version
dependency is real and must match the known-good 580 baseline).
WORKING BOOT+TEST (host): pkill -9 -f "qemu-system-x86_6[4]"; sleep 5; verify none;
  rm -f /opt/nvkvm-guest/mode2-overlay.qcow2;
  NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_qemu.log 2>&1 & disown;
  wait for ssh -p 2223; then stage tests from nvkvm_src 9p (/mnt/nvsrc) + re-add libcuda.so symlink
  (ln -sf /usr/local/nvidia-guest/lib/libcuda.so.580.159.04 /usr/lib/x86_64-linux-gnu/libcuda.so;
  ldconfig) and run scripts/mode2_diag/<test>_run_guest.sh. FRESH boot per GPU test.

GOTCHA (this rebuild): the FIRST provision-boot launch died because the heredoc that wrote
/tmp/boot_provision.sh was in the SAME command as `pkill -9 -f qemu-system-x86_64` — pkill's regex
matched (and the session churn meant) the script never got written, so setsid launched a nonexistent
path. Fix: write the boot script to a PERSISTENT path (/opt/nvkvm-guest/boot_provision.sh) in a
SEPARATE command from any pkill, use the `[4]` regex, then launch.

Prior rebuild log (box 18577, for reference) preserved below.
---

## PRIOR REBUILD (box 70.30.158.46:18577), driver 575.51.03, kernel 6.8.0-59.
Goal: rebuild Mode-2 bench to reproduce bug #12.

## Phases
- [x] 1. Host deps DONE
- [x] 2. Host driver 580.159.04 open DONE (nvidia-smi 580, license Dual MIT/GPL, /usr/src/nvidia-580.159.04 + firmware present)
- [ ] 3. Build QEMU (nvkvm-gpu-emul)
- [ ] 4. Build stub
- [ ] 5. Guest disk build (kernel 6.8.0-117, nvidia 580 open modules, libcuda)
- [ ] 6. Mode-2 smoke test (cuInit)
- [ ] 7. Reproduce #12 (cupctx2_min CTX2 hang)

## Log
- Phase 1 STARTED: apt deps.

- Phase 1+2 DONE. host-libs-580 bundle staged. DKMS tree flat (nvidia/nv-kernel.o_binary + top Kbuild), buildable, shared as ogkm 9p tag.
- Phase 3 STARTED: build QEMU (bg).

## Phase 3 (QEMU) DONE - success criterion 1 MET
- /opt/qemu-nvkvm/bin/qemu-system-x86_64 built; -device nvkvm-gpu-emul,? lists m2fwd/m2exec/m2cefwd/etc.
- build_qemu.sh had 3 bugs (had to fix in deployed /opt/qemu-src tree):
  1. meson configure failed: missing python3-tomli/python3-venv (apt install fixed).
  2. script runs `ninja` from /opt/qemu-src but build.ninja is in ./build -> ran ninja from /opt/qemu-src/build manually.
  3. header copy incomplete: nvkvm .c/.h include ../../src/common/{nvkvm_abi,nvkvm_isolate_proto,nvkvm_ring}.h + abi/*, script only copied 3 headers & rewrote 3 include patterns. Copied ALL into nvkvm_inc/ and sed-rewrote every ../../src/{common,abi}/*.h -> nvkvm_inc/.
  4. virtio.c patch inserted [50]="virtio-nvgpu" into the LAST `};` (virtio_device_info TypeInfo) not virtio_device_names[]. Fixed: put entry after [VIRTIO_ID_GPIO].

## Phase 4 (stub) DONE
- make -C src/stub nvkvm_stub -> install -D /usr/lib/nvkvm/nvkvm_stub (156656 bytes).

## Phase 5 (guest disk) IN PROGRESS
- ubuntu-24.04.qcow2 (+30G) booting on distro qemu (port 2222), key auth works. kernel currently 6.8.0-124 -> must pin to 6.8.0-117.
- seed.iso gotcha: first user-data had literal backslashes in passwd (heredoc \$ escaping) -> cloud-init failed key install. Rewrote via base64, new instance-id, fresh qcow2 -> keys work.

## Phase 5 (guest disk) DONE - criterion 3 MET
- guest ubuntu-24.04.qcow2: kernel PINNED 6.8.0-117 (grub default + apt-hold + unattended-upgrades disabled).
- nvidia 580.159.04 OPEN modules built in-guest vs 6.8.0-117 headers, vermagic 6.8.0-117, staged /home/ubuntu/nvmods/{nvidia,nvidia-uvm,nvidia-modeset,nvidia-drm}.ko
- libcuda 580.159.04 staged /usr/local/nvidia-guest/lib/ (+ libnvidia-allocator, ptxjitcompiler; ld.so.conf.d ahead of system) + libcuda.so dev symlink for -lcuda.
- cuda.h (12.6 cudart redist) -> /usr/include/cuda.h so gcc/nvcc can build tests.
- built .run in-guest (no 9p in plain boot); guest powered off clean to persist qcow2.
- NOTE: run_mode2_vm.sh boots a PERSISTENT overlay mode2-overlay.qcow2 on this base + shares ogkm=/usr/src/nvidia-580.159.04, nvfw=firmware.

## Phase 6 (Mode-2 smoke) DONE - criterion 4 MET
- Boot: pkill qemu; rm mode2-overlay.qcow2; NVKVM_M2CEFWD=1 nohup bash scripts/run_mode2_vm.sh (SSH 2223, ssh vg works).
- Emulated GA106 at 00:07.0; host isolate spawned + MEMTEST data-plane PASS; guest booted 6.8.0-117.
- Loaded stock /home/ubuntu/nvmods (unbind distro nvidia from 00:07.0 first), mknod nodes, insmod nvidia.ko + nvidia-uvm.ko (uvm_maj=234).
- cup2 rc=0: cuInit OK, 1 dev RTX 3060 compute 8.6 11909MiB, cuCtxCreate OK, cuMemAlloc OK, CE HtoD/DtoH byte-exact PASS.

## Phase 7 (repro #12) IN PROGRESS
- Need FRESH boot (GSP WPR2) before cupctx2_min.

## Phase 7 (repro #12) DONE - criterion 5 MET  ***BUG #12 REPRODUCED***
cupctx2_min output (ITERS=2, timeout 180s):
  CUPCTX2_MIN iters=2 (create->destroy only, NO compute)
  [CTX1] cuCtxCreate...
  [CTX1] CTX OK
  [CTX1] cuCtxDestroy...
  [CTX1] CTX DESTROY OK
  [CTX2] cuCtxCreate...      <-- HANGS HERE
  === cupctx2_min exit rc=124 (124=timeout/hang) ===
=> CTX1 create/destroy OK, CTX2 cuCtxCreate hangs = the #12 2nd-context hang, exactly as MEMORY predicts.

## KEY GOTCHA FOUND (cost several runs):
- run_mode2_vm.sh uses `exec qemu`; a stale Mode-2 qemu from a prior boot kept port 2223 +
  held the base qcow2, so "fresh" boots silently landed on the OLD wedged guest (distro nvidia
  had auto-bound+been unbound -> "Failed to enable MSI-X / No interrupts" -> cuInit=101).
  FIX: before each fresh boot, `pkill -9 qemu-system-x86_64; sleep 5; VERIFY ps shows no
  qemu-system` (kill by explicit PID if pkill races), THEN rm overlay + launch.
- Baked distro-nvidia blacklist into base qcow2 (/etc/modprobe.d/nvkvm-blacklist.conf) so the
  emulated GPU at 00:07.0 is pristine (no MSI-X wedge) when our hand-loaded nvmods first attach.

## WORKING BOOT + REPRO COMMANDS (host):
  pkill -9 qemu-system-x86_64; sleep 5; (verify no qemu); rm -f /opt/nvkvm-guest/mode2-overlay.qcow2
  NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_qemu_launch.log 2>&1 &
  # wait for ssh -p 2223 ubuntu@localhost   (or `ssh vg` from the dev box)
  ssh -p 2223 ubuntu@localhost bash -s < /tmp/repro12_clean.sh   # loads nvmods, builds+runs cupctx2_min
  (repro12_clean.sh staged at host /tmp/repro12_clean.sh; smoke test = /tmp/mode2_smoke.sh runs cup2 rc=0)

## ALL SUCCESS CRITERIA 1-5 MET.
