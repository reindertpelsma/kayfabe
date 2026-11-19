# hostabi_595_rows_20261118: PR #522 (host-ABI rows for the 595 GSP branch) on hardware

**STATUS: LIVE (measured), 2026-11-19.** Text evidence of one Vast run: kayfabe PR #522 against its parent, on a
580.159.04 host and on a 595.84 host, with the repo's standard gates and a guest-reboot check. Filtered logs only (no
GPU UUIDs, no addresses, no secrets).

## What was run

| item | value |
|---|---|
| PR under test | #522, branch `claude/hostabi-595-rows-20261118`, head **8a4bd4b0b9c85daf970fc4cc659eadea0ec68824** (head when the run started; the branch has since moved, see *Not covered*) |
| pre-fix control | **f2f009a8c5462038d8fe51e108c947a84cf0298b** = `next` head and the PR's parent. The brief named `a76d13979`; the PR's parent was used instead because it isolates the PR's own diff (`a76d13979` is older `next`) |
| box | one Vast KVM box, label `kf-accept-hostabi1` (instance id 61341573, destroyed, absence verified), 1x RTX 3090, die **GA102** (pci 10de:2204), Ampere, 24 GB, 18 vCPU, 39 GiB RAM, guest kernel 6.8.0-59 |
| dates | rented 2026-11-18 23:52Z, torn down 2026-11-19 02:06Z; cost about 0.55 USD (the cap was 8) |
| hosts | **580.159.04 open** (the provisioned default) and **595.84 open**, swapped on the same box with `provision_host_driver.sh` (`HOST_DRIVER=595.84`, `OPEN_MODULE=yes`) |
| guests | thin suite and CUDA ladder: fast guest / bench image with **580.159.04**; extra matched pair: fat guest **595.84** (`stage_fat_guest.sh`) on the 595.84 host |
| driver | `scripts/drivermatrix/sweep.sh`, one tag per (revision, host); rows: bare-metal cup2, gates, canary, thin (30 arms, budget 180), CUDA ladder (cup2, cup3, cup8, cup8bench) |

**Deviation from the brief, on purpose.** The brief asked for offers whose *listed* host driver is 580.x / 595.84. A
`vms_enabled` Vast box is a VM with the GPU passed through; the host driver is installed inside it by
`provision_host_driver.sh` (the box shipped 575.51.03; the listed offer driver is irrelevant).
So one cheap KVM box and a driver swap covers both hosts; no offer filter by driver
was needed. The 595.91.07 host is the pc's and is covered by another job.

## Results (all at 8a4bd4b0 unless the row says base = f2f009a8)

| host | rev | gates | thin | ladder (580 guest) | ladder (595.84 guest) |
|---|---|---|---|---|---|
| 580.159.04 | PR 8a4bd4b0 | 9/9, gate10 PASS | 30/30 | 4/4 | n/a |
| 580.159.04 | base f2f009a8 | 9/9, gate10 PASS | 30/30 | 4/4 | n/a |
| 595.84 | PR 8a4bd4b0 | 9/9, gate10 PASS | 30/30 | 4/4 | 4/4 |
| 595.84 | base f2f009a8 | 9/9, gate10 PASS | 30/30 | 4/4 | 4/4 |

`KF3_BUILD_RC=0` for both revisions (`kf3-bins/8a4bd4b0`, `kf3-bins/f2f009a8`, see `*/kf3_built.txt`). Thin and ladder
are not discriminating for this PR: the base also passes them (the refusals below do not stop the CUDA ladder here).

## HOST-ABI REFUSED lines (the five controls 0x2080852e / 853a / a618 / 853b / 852a)

Counts are per QEMU instance, full text in `*/host_abi_refused_{thin,ladder}.txt`.

* **580.159.04, PR and base identical:** only `0x2080852a: body is 1716 bytes, its layout is 1784` (every instance).
  **852e / 853a / a618 / 853b: none.** So the PR does not change the 580 behaviour; but 852a is **not "none"** on the 580
  host (before and after): the 580.159.04 guest of this bench asks it with 1716 bytes.
* **595.84, base f2f009a8:** `0x2080852e: host control 0x2080852e has no public header ... its layout at host driver
  595.84 was never measured` (every instance, thin and ladder, both guests). The PR's target refusal.
* **595.84, PR 8a4bd4b0: 852e is gone, but `0x2080853a: body is 1456 bytes, its layout is 15352` is refused in every
  instance, with the 580 guest AND with the matched 595.84 guest** (thin 31/31 instances, ladder 4/4 per guest).
  853a / a618 / 853b are therefore **not** answered at 595.84 by this revision in this bench; a618 and 853b never show
  up (inference: the ask sequence stops at the first refusal, not verified). Who sends 1456 with a 595.84 guest is not
  determined here (1456 is the 580-branch size in `gssreplay::ROWS`).
* Also refused at 595.84 on both revisions (not in the five, unchanged by the PR): 0x20809001, 0x20809009,
  0x20809064, 0x2080a026, 0x2080a028, 0x2080a084 ("has no public header ... never measured").
* No `NOT complete` device-reset line in any log; the guest-side NVRM 0x56 lines in `reboot/` are the same on 580.

## Guest reboot inside the same QEMU (coordinator's added check)

`scripts/reboot_hook.sh` as `POST_CAPTURE_HOOK` of `boot_capture.sh` (via `scripts/reboot_lane.sh`): cold CUDA smoke
(`cup3`: cuInit, cuCtxCreate, module load, a tiny kernel, graded on `KERNEL rv=43`, 120 s bound), then
`systemctl reboot` in the guest with QEMU left running (device reset), wait for a new boot id, smoke again.
No existing in-QEMU reboot lane was found in `scripts/` (grepped); this hook is the local one.

| host | guest | rev | cold | after in-QEMU reboot | guest back |
|---|---|---|---|---|---|
| 580.159.04 | 580.159.04 | PR | PASS | **PASS** | 23 s |
| 580.159.04 | 580.159.04 | base | PASS | **PASS** | 23 s |
| 595.84 | 580.159.04 | PR | PASS | **PASS** | 23 s |
| 595.84 | 580.159.04 | base | PASS | **PASS** | 23 s |
| 595.84 | 595.84 | PR | PASS | **PASS** | 25 s |
| 595.84 | 595.84 | base | PASS | **PASS** | 24 s |

No hang, no retry. The device reset lines (`device reset #2/#3 COMPLETE`, `GSP Halted -> Cold`) are in
`reboot/rbres_*.txt`. The first attempts of the 595.84-guest lane (`rb_*_g595_*`, "RB_FATAL push failed") failed in the
hook only (`/home/ubuntu/cup3.c` is a directory in that image); they were rerun with another path as `*_g595b_*`. The
580-guest lanes used the first hook version (`/home/ubuntu/cup3`), the `g595b` lanes the corrected one (`rb_cup3`); the
logic is the same.

**Not a reproduction of the pc's hang.** The pc is an AD104 (RTX 4070) at 595.91.07 with a Rocky 9.7 guest; this box is
GA102, hosts 580.159.04 / 595.84, Ubuntu fat guest. A pass here says the reboot path works for these cells, not that the
pc's hang is not real (different die, host driver and guest OS).

## Not covered

* The PR branch moved while this ran: head is now `b5d951044` (after `c63b19fb3` and `8a4bd4b0b`). The two later
  commits change `crates/kf-abi/src/hostabi.rs` (+14), `crates/kf-host/src/lib.rs` (+241, tests per the commit titles)
  and docs; they were not run on hardware here.
* Host 595.91.07 (the pc), other dies, 590/610 hosts, Windows guests.
* 580.65.06 / 580.95.05 / 580.105.08 hosts (only 580.159.04 of the four 580.x points was run, one die).
* `a76d13979` itself (see above).

## Layout

* `hostabi_{pr,base}_{580,595}`, `hostabi_{pr,base}_595g595`: `sweep_summary.txt` (the sweep's own result lines),
  `gates_*.log` (filtered), suite and ladder results, `host_abi_refused_*.txt`, `kf3_built.txt`. `595g595` = host 595.84
  with the 595.84 guest (ladder only).
* `reboot/`: the six reboot lanes (plus the two failed hook attempts).
* `box_facts.txt`, `scripts/` (the box-side drivers, hook and collector used; text).
