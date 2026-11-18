STATUS: LIVE, 2026-11-18 (measured on the pc 2026-11-18 with kf3 `6bfa2dbc2`; the hypotheses and falsifiers in section 5 were written BEFORE the runs, results in section 6; open items in section 9; nothing here is sealed or uploaded).

# A vendor installer media as an application layer on the sealed Windows 11 + NVIDIA base (private image, offline, recorded clock)

Scope: how a 23 GB vendor installer media (a public download, a CAE suite's student edition: Fluent, Mechanical/MAPDL, Discovery) reaches the **offline** guest of the blank Windows 11 + NVIDIA base image (`V3_WINDOWS_BASE_IMAGE.md`), how it is installed unattended, what it costs, and how it behaves on first launch, on the GPU, under a second machine identity and in offline throwaway clones. The repository holds no vendor binary; the media and the installed disk stay private on the pc. The guest never has a route out (`restrict=on`), and the clock of every boot after the build is the recorded one, exactly as for the base.

Labels: MEASURED = a command run on the pc on 2026-11-18, evidence on the coordinator (`/workspace/bench/ansys_img_20261118/`); CHECKED = read in the media's own documents (`util/installHelp.pdf`, `util/sysreq_win.pdf`, `manifest/`); CODE READ; INFERRED; DESIGN INTENT.

## 1. Media delivery without a network

The zip (23,036,264,185 B, sha256 recorded when it was downloaded; 459 files, mostly one 7z archive per component) is extracted on the pc into a new empty directory (`unzip`, every CRC checked, the extracted tree compared with the zip's central directory: names and sizes), a UDF + ISO 9660 + Joliet image (`genisoimage -udf`, label `ANSYS_MEDIA`, 23.1 GB) is built from it, mounted read-only and compared again (file list, sha256 of `setup.exe` and the three largest files), and the extracted tree is deleted. The disc is attached to the guest on the AHCI CD port, read-only, through the harness's existing `data_iso` argument of `propimage.build_argv`. MEASURED: in the guest the volume is UDF with 459 files and 23,116,458,295 B (the zip's uncompressed total), and the sha256 of `setup.exe` and the three largest files equal the host's. The zip itself is only read (no repack). Peak extra disk on the pc: about 70 GB for 15 minutes (zip + tree + image).

## 2. The install recipe

CHECKED (`installHelp.pdf`, chapter "Silent Mode Operations", Windows part): `setup.exe -silent -install_dir "C:\Program Files\ANSYS Inc" <product flags>`; "if you do not specify a product flag, all products will be installed"; product flags used here: `-fluent -mechapdl -discovery`; the guide has no separate agreement flag (`-console` only displays the agreement text), so the silent form is the way the owner's acceptance of the vendor's end-user agreement is applied for this image. The install log is `install.log` / `install.err` in the install directory.

The command runs **in the signed-in desktop session** as a one-shot scheduled task (interactive, highest run level; the same mechanism `propimage_check.task_wrapper` uses for application checks), started from ssh, so that child helpers have a desktop and an elevated token; the harness polls a done marker and the product's own log instead of holding one ssh call for 20 minutes.

MEASURED: setup exit code 0, 19 min 36 s from start to exit (`01:16:54Z` to `01:36:30Z`), the product's own "Installation Complete" at `01:35:01Z`; offline, 8 vCPU, 8 GiB. `C:` went from 131.3 GB free to 65.3 GB free: **66 GB installed** (`C:` used 93.6 GB of 158.9 GB). `install.log` is 232 lines with 0 matches for http, token, account, sign-in, activation, proxy; the Windows DNS cache was empty afterwards. `install.err` and the last line of `install.log` say "completed with warnings/errors": two components named by the media's manifest are **not on the media** (`tp/cudss`, `python_site_syscplg`; the zip has no such entries; the manifest file for Discovery is dated 2026-03-17, the other 777 entries 2026-02-24), so the installer's 7z call failed on a missing file (exit 2). The Discovery libraries still contain their own cuDSS runtime; what the missing `tp/cudss` component would have added was not established (INFERRED: nothing in the checks below needed it).

A Windows Firewall dialog ("allow this app on public and private networks") appears on the first start of the licensing helper that every product starts; it is modal, topmost, and one is stacked per start. The recipe sets `Get-NetFirewallProfile | Set-NetFirewallProfile -NotifyOnListen False` once (MEASURED: no dialog afterwards, products unaffected).

## 3. First launch (offline) and what a launch check can prove

| Product | Command in the guest | MEASURED |
|---|---|---|
| Fluent | `fluent.exe 2d -g -t2 -i exit.jou` (journal: `/exit yes`) | starts, banner and transcript written, exits in 37 s; no prompt of any kind |
| MAPDL | `ANSYS261.exe -b -np 2 -i small.inp -o small.out` (a 13k-node static solve) | solves, "NUMBER OF ERROR MESSAGES ENCOUNTERED= 0", 29 s |
| Discovery | `Discovery.exe` | a "Welcome" window (starting edition, theme, view controls, graphics mode) with one **Launch** button must be clicked, then the main window opens after 65 to 80 s; the harness clicks it through UI Automation (`InvokePattern`); no sign-in is required; a second Discovery process while the first runs is refused by the product (one instance at a time) |

The Discovery welcome window re-appeared on every start in these runs (the first process was killed, not closed, so a setting may not have been written); a documented switch or setting that skips it was looked for (the libraries carry a `ShowWelcomeScreen` user setting and a welcome-screen check box in the UI) and not settled: DESIGN INTENT keeps the UI Automation click until a setting is shown to work.

## 4. GPU

- **MAPDL `-acc nvidia`:** stops with the product's own fatal message that no *recommended* GPU device was detected, although CUDA enumerates the RTX 4070. CHECKED in `sysreq_win.pdf`: "recommended" is a fixed table of cards (the 4070 is not in it) and the document names an override, `ANSGPU_OVERRIDE=1`. MEASURED: the message is the same with the guest seeing 4096, 8192 and 10240 MiB of video memory (so it is not the window size; 11264 and 11000 MiB are refused by kf3 on this host, see section 8); with `ANSGPU_OVERRIDE=1` MAPDL runs and says "This solution uses the GPU accelerator capability" with one warning (3.7 GB on the card, 15 GB recommended).
- **Fluent native GPU solver (`-gpu`):** the product does not start it on this die (it refuses at node spawn; the vendor message and what it checks are in the private note). The older `-gpgpu=1` flag launches.
- **Discovery:** host-authored census of the kf3 log (the judge of PR #475, `scripts/bench/gpu_use.py`, window = the kf3 status lines between two marks): idle desktop 31 submissions/s; Discovery starting and showing its main window 382 submissions/s; GR0 engine wakes 4 versus 9079 in windows of 40 s and 85 s; Xid 0, no dead channel in any window. This shows Discovery's graphics work reaching the GPU through kf3; it is not a solve.

## 5. Hypotheses and falsifiers (stated before the runs)

| # | Hypothesis | Falsifier |
|---|---|---|
| M1 | The media extracts and mounts intact on the host | any CRC, name, size or hash difference |
| M2 | The guest sees the same files on the disc | any difference in count, bytes or hash |
| I1 | The silent install finishes offline with exit code 0 within 120 min | non-zero exit, no product tree, or 120 min |
| I2 | `C:` keeps at least 40 GB free (so 40 to 60 GB more apps would fit without a resize) | less than 40 GB free |
| L1 | Fluent and MAPDL start and finish a batch run offline | either refuses or prompts |
| G2 | Discovery work is visible in the host census, no Xid | no census delta, or an Xid |
| C1 | Two offline clones with the recorded clock pass the harness rows and the launch checks | any failed row |
| ID | The same disk booted with a second machine identity (SMBIOS, MAC, disk serial, GPU UUID) still launches the products | any product refuses or prompts |

## 6. Results

M1 HELD (UDF view; the ISO 9660 view was not mounted because of a wrong mount option in the scratch script). M2 HELD. I1 HELD (19 min 36 s). **I2 FALSIFIED: 65.3 GB free**, so another 40 to 60 GB would not fit without moving the recovery partition (it sits at the end of the disk, after `C:`) and growing the volume; the expected size of a sealed copy was not measured (INFERRED from the 23 GB media holding 66 GB installed: about 30 to 35 GB compressed on top of the base's 11 GB). L1 HELD. G2 HELD (section 4). C1 HELD, 2 of 2: `propimage_check.py --apps none` power OK, GPU PASS, verify PASS; no route out (TCP 443, DNS and HTTP fail, no default route); 0 problem devices; Kernel-Power 41 count 0; clock inside the recorded window; Fluent, MAPDL and Discovery launch checks pass in both; census Xid 0. ID HELD: the guest saw the second identity on every hardware row and not the first; the TPM endorsement key and the OS machine id were deliberately not changed (the same TPM blobs and disk), so this tests hardware-id binding, not a new Windows install.

Authenticode of the media's own installer executables (`setup.exe`, `ProductConfig.exe`, `Uninstall.exe`, `instcore.exe` and the prerequisite installers): the signed digest and the signature chain verify (host side, the `signify` library; its optional page-hash check fails for every signed file it is given, third-party ones included, so it is treated as a library artefact: INFERRED). 74 of the 78 files inside the installer-core archive are unsigned third-party libraries (Qt, curl, zlib).

## 7. Harness findings

- The scratch driver pattern of the base build (one script per step, `PROP_DIR` hard-coded in the helpers) was copied once more, with image-agnostic paths and arguments only; folding the drivers into one tested tool remains the follow-up named in `V3_WINDOWS_BASE_IMAGE.md`.
- `propimage_check.py --hold-until-file` still ends with `rc=1` for the `hold` row's verdict `released` (known).
- A single `ssh` PowerShell call longer than about 3 minutes is a trap (a foreground wait inside the tool call times out and a half-run script keeps running in the guest): every long guest step is a scheduled task with a done marker.
- `kf3` refuses a video-memory window of 11264 MiB on the 12 GiB card while the desktop holds 323 MiB (realize: `NoMemory`) and 11000 MiB (`CUDA_ERROR_OUT_OF_MEMORY` at context creation); 10240 MiB works.
- `qemu` of the kf3 build has no PNG support for `screendump` (PPM works); the kf3 device has no framebuffer for `-vga none`, so a screendump of a held clone shows nothing.

## 8. What was not done

No seal, no pack, no upload: the working disk and its throwaway overlays stay on the pc. No other application was installed. The Fluent GPU solver was not chased further.

## 9. Open items

1. The layer's installed size (66 GB) decides whether it is baked into a shared image or installed at run time from the media; both are possible with the recipe above (DESIGN INTENT: the config chooses per application).
2. The Discovery welcome window: find the setting that skips it, or keep the click.
3. Re-measure the media completeness with a newer download (two components missing from this zip).
4. A fresh Windows install with the layer on top (a new TPM and machine id) was not tested.
