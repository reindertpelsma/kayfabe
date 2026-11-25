STATUS: LIVE, 2026-11-24. Which Adobe AI/GPU features of the sealed image B work offline with the revoked login, and why the failing ones fail. One Vast box, one boot (`g1`). Hypotheses and falsifiers were committed before the boot (`PREREGISTRATION.md`, commits 3581ba074 and c4a632a32). Labels: MEASURED (a file of this directory or a line of this run), INFERRED (not tested).

Run: 2026-11-24, box 62286289 (RTX 4090 24564 MiB, AMD Ryzen 9 7950X, 48 GiB, offer search by total price), rented 03:34Z, destroyed 05:44Z. Branch `claude/imgb-offline-features-20261124`; kf3 QEMU built on the box from 3581ba07 (`box/provision.log`, `KF3_RC=0`), harness scripts at d6b204dc8 plus the Premiere probe added during the run (`scripts/bench/windows/imgb_offline/`). Host driver 580.159.04 (kayfabe's in-VM host driver), guest driver 580.88. Guest RAM 16384 MiB, guest FB 8192 MiB, virtual sound card (the overrides of the 2026-11-20/22 runs). Image `kfimg-adobefull2-20261122T015401Z` (private R2 object; never in git, never copied back).

# Transfer and integrity (MEASURED on 2026-11-24, box 62286289, `box/`)
- Presigned GET minted on the coordinator with the read-only remote (`presign_get.py` with `r2ro`: the `r2` token's URL answered 403 from the box, as the release kit README warns), file mode 0600, shredded on the coordinator and on the box right after the fetch.
- `image.qcow2` member (offset 61, 96,514,746,368 B) by ranged GET in 1171 s at 82.4 MB/s; sha256 equals the pin `ec36a395...ad796` (`member_download.log`).
- Zip pin: `zip_verify.py` fetched the 61 header bytes and the 737,629 tail bytes, hashed header + member + tail: equals the pin `c702ef2d...e7776`; `state.json` and `manifest.json` extracted from the tail match `manifest.json`, whose `image.qcow2` entry equals the member pin (`zip_verify.log`).

# Boot g1 (MEASURED, `box/harness_g1_rows.txt`)
Offline (`restrict=on`), recorded clock: RTC base 2026-11-22T02:43:26Z, guest UTC 02:47:37Z at the first probe (about 53 minutes after the seal). `gpu` and `verify` PASS; host Xid CLEAN (0 before, 0 after, readable ring buffer). The guest stayed up 1 h 40 min and ended with the harness's own stop: the only QMP lifecycle event is `SHUTDOWN guest-shutdown` (PR #578 recorder). The 45-minute guest death of 2026-11-22 did not happen. Guest System log for the whole boot: no TDR (4101), no `nvlddmkm` event, no Kernel-Power 41, no LiveKernelEvent report, no Application 1000/1001/1002 (`g1/guest_events_whole_boot.txt`). kf3: no walk CUT, no CEILING line, no DEAD; 256 `VA_SLOW` lines (`g1/kf3_log_counts.txt`). No sign-in dialog appeared in any application.

# Per feature (offline, recorded clock, revoked login)
| application | feature | result | reason class / evidence |
|---|---|---|---|
| Camera Raw 19.0 (in Photoshop, real DNG) | Masking > Background | FAILS within 8 s: "Unable to load AI selection model" (click 02:52:27Z) | ENTITLEMENT through the network (below); `g1/acr_select_background_error.png` |
| Camera Raw | Masking > Subject | FAILS, same message (02:58:59Z and 03:21:15Z) | same; `g1/acr_select_subject_error.png` |
| Camera Raw | People (automatic detection in the masking panel) | FAILS: "Unable to select people in this photo." | same; `g1/acr_people_unable.png` |
| Camera Raw | Masking > Sky | WORKS: mask created (`sky inference: time 19.85 sec` in the Camera Raw log) | model in ProgramData ModelZoo; `g1/acr_sky_mask_ok.png` |
| Camera Raw | Lens Blur > Apply | FAILS: "Unable to load Lens Blur AI depth estimation model." (03:11:31Z) although `depth_estimation::DoInference` ran in the same session (36.8 s, 39.7 s, 47.5 s) | same class; `g1/acr_lens_blur_error.png`, `g1/camera_raw_log_g1_filtered.txt` |
| Camera Raw | GPU self-test (new die, so the 2026-11-19 cached failure was retested) | PASSES under kf3: `Completed cr_gpu_test::RunSanity successfully`, GPU3/GPU4 hard status success, `gpu_compute_quick_self_test_passed="True"` | `g1/camera_raw_log_g1_filtered.txt`, `g1/camera_raw_gpu_config_after.txt` |
| Camera Raw | Denoise, Super Resolution | NOT RUN here (Denoise COMPLETED under kf3 on the pc on 2026-11-24, `traces/pc_camera_raw_20261124/`, PR #587) | - |
| Photoshop 28.0 | Select Subject (`autoCutout`, scripted) | WORKS: selection 1347,1391 to 1748,1505 px, 36.3 s first, 7.8 s second | bundled `Required/sensei_models`; `g1/ps_ai_selection_jsx_result.json` |
| Photoshop | Select Sky (`selectSky`, scripted) | WORKS: selection 0,0 to 4480,975 px, 14.4 s | same |
| Photoshop | Remove Background (`removeBackground`, scripted) | WORKS: layer mask made (8.5 s) | same |
| Photoshop | Remove tool | WORKS in non-generative mode only: toast "We used non-generative technology for this result. To use generative AI with the Remove tool, please check your internet connection and try again." | NETWORK (the 5.6 GB on-device generative store was not used); `g1/ps_remove_tool_nongenerative_toast.png` |
| Photoshop | Neural Filters > Colorize | WORKS: colorized preview | installed model; `g1/ps_neural_colorize_ok.png` |
| Photoshop | Neural Filters > Landscape Mixer | INCONCLUSIVE: preset selected, strength 100, no visible preview change after 60 s, no message | - |
| Photoshop | Filter > AI Denoise | cloud partner model: a "Start using partner models" consent dialog (Gemini, Topaz, Flux); not accepted | NETWORK/ENTITLEMENT (cloud); `g1/ps_ai_denoise_partner_models_dialog.png` |
| Photoshop | Object Selection, other Neural Filters, Generative Fill | NOT RUN | - |
| Premiere Pro 27.0 | launch | WORKS: home window after 55 s, no sign-in dialog | - |
| Premiere Pro | open the recent "Preview-Your-Clips" Learn project | CRASH 03:30:51Z: "Unhandled exception!" on thread `ORT_AudioClipAudioFP16-20250912` (an ONNX Runtime audio model thread), renderer CUDA | UNATTRIBUTED; `g1/premiere_crash_threads.txt` |
| Premiere Pro | new project + the 1080p trailer clip as a sequence (scripted), then Scene Edit Detection | CRASH 2 to 3 s after the sequence was shown, 3 of 3: with Scene Edit Detection (04:06:14Z) and with a renderer switch that did not take (04:18:44Z) the report says "Unhandled exception!" on `Display Thread`, renderer CUDA; with no Scene Edit Detection at all (04:22:01Z) the crash time was read but not its thread | UNATTRIBUTED, kayfabe NOT excluded (below) |
| Premiere Pro | Object Mask, Scene Edit Detection, Speech to Text en/nl, Enhance Speech | NOT REACHED: every sequence crashed first | - |
| Lightroom Classic, After Effects, Illustrator, Substance Sampler | all listed features | NOT RUN (time box) | - |

# Why the Camera Raw AI selections fail (part 2)
MEASURED in boot g1 on 2026-11-24 (`g1/ngl_photoshop_camera_raw_token_excerpt.txt`, from the guest's licensing log inside Photoshop):
- While Camera Raw is open it asks for an IMS access token about every 5 s: `GetImsAccessToken: invoked clientId: CameraRaw clientScope: openid,firefly_api,ims_cai.social.read,ims_cai.verifiable_credential`. 38 requests in the 4-minute copy.
- Every one misses the cache (`FetchCachedAccessToken returned : 45`, 38 of 38).
- The fetch then goes to the network: `Calling endpoint https://ims-prod06.adobelogin.com/ims/token/v4`, `MapLastErrorToStatusCode: Error is 12007` (WinINet: name not resolved), `ImsAccessToken Status: Network Error` (NGL status 102). After repeated failures NGL throttles itself (`IMS_ERROR_CLIENT_THROTTLING`, status 326). At the Subject click (03:21:15Z) the request at 03:21:14Z ended in 326.
- The Camera Raw log of the same session (`g1/camera_raw_log_g1_filtered.txt`) shows `*** Error: ML model not loaded ***` after each failing action, each followed by `No background download is running.`. It names no missing path.
- The model files are on disk: `select_subject_v5_on_device` and `metacaf/background_people_v1` in the per-user CloudDownload cache (`g1/camera_raw_clouddownload_listing.txt`), the bundled set in ProgramData (`g1/camera_raw_programdata_modelzoo_dirs.txt`).
- The guest's DNS client log shows Photoshop querying `ims-prod06.adobelogin.com` every 1 to 2 s from before the first click (`g1/dns_queries_click_windows.txt`): the token traffic is continuous, not only at the click.

Classification: ENTITLEMENT that needs the network. The selection and depth models are present. Camera Raw gates loading them on a fresh IMS token for its own client and the `firefly_api` scope, and the image holds no cached token for that scope at any clock. INFERRED: the gate is the cause, not a co-occurrence. The basis is that the same models loaded online and signed in on 2026-11-21, no other reason is logged, and the Sky model, which has no such failure, loads. Not measured: what Camera Raw does with the token (a decryption key or an entitlement flag). Consequence (INFERRED): with the login revoked on the server, these features cannot work offline at any guest clock, and they cannot work online either without a new sign-in into a copy of the image.

# Hypotheses (PREREGISTRATION.md)
- H_clock: FALSIFIED. With the recorded clock, about 53 minutes after the seal, Select Background and Select Subject fail with the same message the pc saw with the real clock. The `--stop-utc` control boot was not needed (preregistered rule).
- H_net: SUPPORTED in the narrow sense. The failing path makes a network request (the IMS token POST) that fails offline. The falsifier "no outbound attempt by the application" did not occur.
- H_ent: SUPPORTED. The failing request is an identity/entitlement token for the CameraRaw client with the `firefly_api` scope. No licence-profile refusal is logged: the NGL profile itself is served from cache.
- H_missing: FALSIFIED. The files are present and no log line names a missing path.
- H_gpu3: FALSIFIED. The GPU self-test passed under kf3 on this die, and masking still failed.
- H_token: REFINED. It is not an expiry. There is no cached token for this client and scope at all (cache miss 45 on every request), so the clock does not matter. The prediction "the recorded clock loads the model" is falsified.
- H_split: SUPPORTED for Photoshop's bundled models (Subject, Sky, Remove Background) and Camera Raw's ProgramData Sky model, which work, against Camera Raw's token-gated Subject, Background, People and Lens Blur, which fail. Not tested for Premiere and After Effects (Premiere crashed first).
- H_kf (part 3):
  - Camera Raw failures: kayfabe EXCLUDED. The reason is an application-level offline token fetch (WinINet 12007), and the message equals the pc's bare-metal VFIO result.
  - Premiere crashes: NOT excluded. Premiere crashes 3 of 3 about 2 s after a 1080p H.264 sequence is shown ("Unhandled exception" on the Display Thread with the CUDA renderer in the 2 reports read). There was no TDR, no GPU event in the guest and no host Xid. No bare-metal arm exists for it yet, and no earlier run of image B ever got a Premiere sequence to play (`traces/imageb_apps_20261116/second_pass/README.md`: the project calls hung). No kayfabe defect is claimed and no test is added. The next control is the same scripted sequence on the pc under bare-metal VFIO (falsifier: it crashes there too, which makes it the application or the image). A second cheap control is a project whose renderer is software-only, set by the GUI: the QE call used here did not switch it.

# What this means for image B
- INFERRED: not coverable offline with the revoked login.
  - Camera Raw and Lightroom AI masking (Subject, Background, People, and most likely Objects and Landscape, which use the same panel; not tested).
  - Lens Blur.
  - The generative Remove.
  - Photoshop AI Denoise and AI Sharpen (cloud partner models).
  - Generative Fill and Text to Vector (cloud, not run).
  The Camera Raw stall question of 2026-11-21 cannot be reproduced on image B at all.
- Coverable offline, as MEASURED here:
  - Photoshop's bundled ML on the GPU: Select Subject, Select Sky, Remove Background, non-generative Remove.
  - Neural Filters Colorize.
  - The Camera Raw Sky mask and the depth-estimation inference (the GPU is busy for 37 to 48 s per run under kf3).
  - Camera Raw's 260 compute pipeline states and its GPU self-test.
  - Camera Raw Denoise (pc, PR #587).
  These are the offline AI workloads to build rows on. The Premiere AI features stay unknown until its sequence crash is settled.

# Cost and teardown
Box 62286289: 2.08 h billed, 1.35 USD in total (GPU 1.17, storage 0.18; `vast_bill.py`), credit 42.72 to 41.37 USD; cap 3.5 USD and 3 h (`kf-imgboff-` entry in the cost guard). Destroyed by its own id at 05:44Z; `vastai show instances-v1` empty afterwards; `vast_guard.py --enforce` clean before and after. No URL, key, account name, e-mail or image content is in this directory. Screenshots are cropped to dialogs or panels; full screenshots and raw logs stay on the coordinator only (a Premiere home screen shows the account's first name).

# Files
| file | content |
|---|---|
| `PREREGISTRATION.md` | hypotheses and falsifiers, written before the boot, with an addendum |
| `box/` | provisioning log, image transfer and verification logs, harness rows and the QMP event, host Xid watch |
| `g1/*.png` | cropped dialogs and panels named in the table |
| `g1/ngl_photoshop_camera_raw_token_excerpt.txt` | the masked licensing-log excerpt (token request, HTTP error, counts) |
| `g1/camera_raw_log_g1_filtered.txt`, `g1/camera_raw_gpu_config_after.txt` | Camera Raw's own log of the session and its GPU config after the self-test |
| `g1/camera_raw_clouddownload_listing.txt`, `g1/camera_raw_programdata_modelzoo_dirs.txt` | the model stores on disk (names, sizes, times) |
| `g1/dns_queries_click_windows.txt` | the guest's DNS client queries (process, name) around the clicks |
| `g1/ps_ai_selection_jsx_result.json` | the scripted Photoshop selections (`scripts/bench/windows/imgb_offline/ps_ai.jsx`) |
| `g1/premiere_crash_threads.txt` | the crashing thread and renderer of each Premiere crash (from Premiere's own crash reports in the guest) |
| `g1/guest_events_whole_boot.txt`, `g1/kf3_log_counts.txt` | guest System/Application events of the boot; kf3 log counts |
