# Video: PASS

MEASURED tool: FFmpeg/FFprobe **n8.1.3-14-g330caae0c1-20261008**, existing offline Windows app-matrix media. Transferred from trusted host to guest over QEMU host-loopback HTTP, no guest internet. SHA256 on both sides matched: ffmpeg `2eb38ea1e422796455fd677e0b5b0f72ac848bc549275dc161919a003712f587`; ffprobe `29d10c60da05f064066be414c9f9a19fde2beb87ce3a30dca1ce8d4f7e481a74`. Temporary lane-only HTTP server stopped after transfer.

MEASURED commands/expected/results: `video.ps1` and `video-result.txt`. Generate synthetic testsrc2 1280x720, 30 fps, 10 seconds. **h264_nvenc** and **hevc_nvenc** both exit **0**; ffprobe `-count_frames` exits **0**, confirms respective codecs, 1280x720 and **300 frames each**. CPU decode and `-hwaccel cuda -hwaccel_output_format cuda -vf hwdownload,format=nv12` decode both exit **0**. Both produce 300 frame hashes; **300/300 identical** with software CPU decode after matching yuv420p output. Hardware-frame download is required by this command. No generic software-decode fallback is accepted as this result. One pass per encode codec and one NVDEC H.264 pass, no repeat spread; stage duration 5.241 s.

MEASURED host Xid **21 -> 21**, delta **0**, guest System 4101/nvlddmkm **0**, boot-only reset req=done=1 unchanged, mirror/channel refusal counters **0**. `video-log-check.txt`: no inspected non-status/non-lifecycle refused/ERROR/TSG_MIRROR/0x1f lines. Pre-existing unsupported GSP query refusal total 1721 -> 1813, distinct set stayed 75. No driver/VM restart or flag change.

CODE READ scope: NVDEC H.264 and NVENC H.264/HEVC are covered; AV1, HEVC decode, Vulkan Video, long video soak and Linux fat-guest video remain UNRUN. No product change; mutation testing N/A.
