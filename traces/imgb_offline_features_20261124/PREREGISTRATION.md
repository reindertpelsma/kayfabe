STATUS: LIVE, 2026-11-24. Written and committed before the first guest boot of this run. The box (instance 62286289, RTX 4090 24564 MiB, label prefix `kf-imgboff-`) was rented at 03:34Z; at the time of writing no image byte had been fetched and no guest had booted. Labels used in the result: MEASURED (a file or log line of this run, named), INFERRED.

# Question
The pc control of 2026-11-24 (`traces/pc_camera_raw_20261124/` on branch `claude/pc-camera-raw-20261124`, PR #587) found that on the image B add-on overlay, booted OFFLINE (`restrict=on`) with the host's REAL clock (2026-11-24) and the Adobe login revoked on the server since 2026-11-22, Camera Raw Select Background, Select Subject and Lens Blur all end within 30 s with "Unable to load AI selection model" (Lens Blur: "... depth estimation model"), on bare-metal VFIO and under kf3 alike, while AI Denoise completed under kf3. On 2026-11-21 the same overlay loaded those models ONLINE and signed in.

This run settles, on the SEALED image (`kfimg-adobefull2-20261122T015401Z`, private R2 object; zip sha256 `c702ef2d...e7776`, `image.qcow2` sha256 `ec36a395...ad796`), booted the way image B is meant to run (offline, `restrict=on`, the RECORDED clock from `state.json` plus `gap_s`, as `propimage_check.py` sets it and as the 2026-11-22 Vast run did):
1. which Adobe AI/GPU features work offline and which fail, per feature;
2. why the failing ones fail, from the applications' own logs and the guest's name-resolution attempts, classified as MODEL_MISSING (a named model path is absent), ENTITLEMENT (a licence, profile or sign-in check refuses), NETWORK_ONLY (the feature needs a network fetch that fails offline), CLOCK (the result depends on the guest clock) or UNKNOWN (no logged reason);
3. whether kayfabe has any part in a failure.

Licensing rule of this run: observe only. Nothing is signed in, no licence file, token, host entry or certificate is changed, no check is patched or bypassed. A sign-in dialog is recorded (screenshot cropped to the dialog) and dismissed. The only clock is the harness' recorded clock; a second boot with a later clock (`--stop-utc`, below) is an observation of the same image, not a change to it.

# Hypotheses and falsifiers (each written before its probe)
H_clock: the pc failure is time-based. The cached licence or model token was still valid around the seal (2026-11-22 01:54Z) and had expired by 2026-11-24, when the pc ran with the real clock.
- Prediction: with the recorded clock (about 50 minutes after the seal) Camera Raw Select Background and Select Subject create a mask.
- FALSIFIED if the recorded-clock boot shows the same "Unable to load AI selection model" message. SUPPORTED only if the recorded-clock boot creates the mask AND a boot with a clock after 2026-11-24 (`--stop-utc`, same image, same box) shows the model-load message again; with only the first half the result is "consistent with H_clock", not "supported".

H_net: the failing features need a network round trip at every load (a signed manifest, a key or an entitlement refresh), whatever the clock.
- Prediction: the failure appears under the recorded clock as well, and the guest's DNS client log shows Adobe host names queried by the application process in the seconds before the error.
- FALSIFIED if the feature works under the recorded clock offline, or if no name resolution and no outbound connection attempt by the application appears in the window from the click to the error.

H_ent: a licence or entitlement check refuses the feature (the cached profile lacks it or marks it invalid offline).
- Prediction: an application or licensing log line (NGL, OOBE, Creative Cloud, Sensei, model-download logs) names entitlement, licence, profile, token, sign-in or authorisation at the time of the error.
- FALSIFIED if the logs written in the error window contain no such line and another reason (a missing path, a network error) is logged instead.

H_missing: the failing features lack a model file in the sealed image.
- Prediction: a log line names a missing path, or the model directory the feature uses is absent.
- FALSIFIED by the read-only inventory of 2026-11-22 for Camera Raw masking and Lens Blur unless a log names another path: the models are on disk (ProgramData ModelZoo and the per-user CloudDownload cache). The result names the path whenever one is logged.

H_split (per-feature prediction): features whose models are bundled in the program directory (Photoshop `Required/sensei_models`: Select Subject, Object Selection; Premiere/After Effects `MLModels` and the `ccd-deferred` ObjectMask store: Object Mask, Scene Edit Detection, Roto Brush; Camera Raw raw Denoise from ProgramData `ModelZoo`) work offline. Features that Camera Raw serves from the per-user CloudDownload cache (Select Subject v5, Select Background, Lens Blur depth) fail. Cloud features (Generative Fill, Text to Vector) fail with a sign-in or network message.
- FALSIFIED if a bundled-model feature shows a model-load failure, or a CloudDownload-cache feature works under the same clock.

H_kf: kayfabe causes a failure.
- On the pc the message was identical on bare-metal VFIO and kf3 (real clock). In this run there is no bare-metal Windows arm (a Vast box cannot boot the guest without kf3), so this run tests H_kf only indirectly: a kayfabe cause is EXCLUDED for a feature when its failure message and logged reason are application-level (licence, network, missing file) and match the pc's bare-metal message. A failure that ends in a GPU error (TDR, LiveKernelEvent 141/117 in the same second, `nvlddmkm` 13/14/153, a host Xid, a `qemu-exit` row with `GUEST_PANICKED`, an Application Error in a GPU module) is NOT excluded and is reported as a kayfabe suspect with its evidence.

# Probes, in order of value per minute (one boot, held by `--hold-until-file`; the guest has died about 45 minutes after boot before, so the order matters)
Before the first probe: the guest UTC; a listing (path, size, mtime) of the log directories (`%LOCALAPPDATA%\Adobe`, `%APPDATA%\Adobe`, `%LOCALAPPDATA%\Temp`, `C:\ProgramData\Adobe`, Creative Cloud and NGL logs); the Windows `Microsoft-Windows-DNS-Client/Operational` log enabled (an observation channel, it changes nothing in the applications).
1. Camera Raw in Photoshop on the real DNG (`raw/dji_fc7303_0765.dng`, data volume): Masking > Select Background, Select Subject, Select Sky; Lens Blur; Detail > Denoise; Enhance > Super Resolution. Each outcome is a cropped screenshot of the panel or message.
2. Photoshop, scripted where Photoshop's own scripting reaches the feature (the error text of `executeAction` is logged verbatim): Select Subject (`autoCutout`), Select Sky, then by UI Object Selection, the Remove tool (on-device), the installed Neural Filters (Colorize, Super Zoom; then Smart Portrait as a predicted MISSING one), Generative Fill (predicted cloud-only).
3. Logs after the Camera Raw and Photoshop probes: every text file under the listed directories written since the probe start, filtered to lines with the probe's time and the words model, load, download, entitle, licen, profile, token, sign, auth, network, http, connect, fail, error, expired, clock; DNS client events with process ids; `Get-DnsClientCache`. Account names and e-mail addresses are masked before anything leaves the box.
4. Premiere Pro: Object Mask, Scene Edit Detection, Speech to Text (English and Dutch packs), Enhance Speech. Lightroom Classic Denoise and masking only if Camera Raw and Lightroom might differ (same engine; Lightroom needs a catalog and an import).
5. After Effects (Roto Brush, Mask Tracker, Advanced 3D), Illustrator (masking, Text to Vector), Substance Sampler (image to material): only if time is left. Anything not reached is NOT RUN in the table, never guessed.
6. If Camera Raw masking WORKS under the recorded clock: a second boot of a fresh overlay with `--stop-utc` set so that the guest clock is after the pc run (2026-11-24 03:00Z or later), repeating probe 1 only (the H_clock control). If masking FAILS under the recorded clock, that second boot is not needed for H_clock and the time goes to probes 4 and 5.

# Stop and report rules
- Each boot records its end: `qemu-exit` row with `GUEST_PANICKED args=0x...` or the shutdown reason (`propimage_check.py`, PR #578), host Xid watch before and after.
- Time box: 3 box-hours and 3.5 USD (cap entry `kf-imgboff-` in the cost guard). The box is destroyed by its own id and its absence is verified.
- A kayfabe defect, if one shows up, gets the smallest GPU-free test that fails before the fix and passes after.

# What is NOT claimed
One box, one die (AD102), one sealed image, one or two boots. A feature that "works" works on this boot; a feature that fails is classified only as far as its logs say. An inference is written as INFERRED.

# ADDENDUM, 2026-11-24 about 03:58Z, still before the first guest boot (the image download was at about 20 GB)
Source: a read-only look at the sealed image on the coordinator (`qemu-nbd --read-only`, `ntfs-3g -o ro,norecover`; unmounted afterwards, image size, mtime and mode unchanged). No content of it is committed beyond the facts below.
- MEASURED: `%APPDATA%\Adobe\CameraRaw\GPU\Adobe Photoshop Camera Raw\Camera Raw GPU Config.txt` (written 2026-11-19 17:39Z, unchanged since) holds `gpu_compute_quick_self_test_passed="False"`. The last Camera Raw log in the image (`Camera Raw Log Latest v1.txt`, host After Effects, RTX 4070, 2026-11-22 01:26Z) says `Need to run gpu self tests : False`, `GPU3 failed (previous) sanity test`, `GPU3 Hard Status: fail_cached_sanity_test`, `GPU4 Hard Status: fail_no_gpu3`, `ML inference: default device (Auto Fastest)`. So since 2026-11-19 Camera Raw runs without its GPU3/GPU4 compute path on that die and does not test again while the GPU digest is unchanged.
- MEASURED: the licensing logs (`%LOCALAPPDATA%\Temp\NGL\NGLClient_*.log`) of the last session show the profile served from cache (`CachedNotValidated`, then `Cached`) and IMS access tokens taken from a cache (`FetchAccessTokenFromDeviceToken : FetchCachedAccessToken`).

Two more hypotheses, written now:
H_gpu3 (the cached failed GPU self-test causes the model-load failure). INFERRED to be unlikely before the run: the file is from 2026-11-19 and the 2026-11-21 online session loaded the masking models with it in place. On this box the die differs (RTX 4090), so the digest changes and Camera Raw should run its self-tests again in the guest, under kf3.
- FALSIFIED if Camera Raw's log in this run shows the GPU3 self-test passing and masking still fails with the model-load message, or shows it failing and masking works.
- Side question, reported either way: does Camera Raw's GPU3 quick self-test pass under kf3 on the RTX 4090? A FAIL is a kayfabe suspect (a GPU compute result that differs from what Camera Raw expects), to be compared with a bare-metal arm before any claim.

H_token (refines H_clock): the masking and Lens Blur models need a valid IMS access token, which the image holds only in a time-limited cache; the pc (real clock, 2026-11-24) was past it, the recorded clock (about 50 minutes after the seal) is not.
- Prediction: under the recorded clock the licensing log shows the cached token used without error around the click and the model loads; a guest clock after 2026-11-24 shows a token fetch that fails offline right before the model-load message.
- FALSIFIED if the recorded-clock boot fails with the model-load message while the licensing log shows a cached token accepted in the same second, or if no token request is logged around the click.
