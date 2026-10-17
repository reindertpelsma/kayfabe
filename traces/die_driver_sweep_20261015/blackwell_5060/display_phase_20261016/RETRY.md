# Second display-phase rental

**STATUS: LIVE, 2026-10-16. Provisioning only; no hardware result.**

Own56286918/verifiedmachine152012/advertisedRTX5060Ti, original175minute
timer2478432, same3hourhardcap. Prior failed56284522 is destroyed/APIabsent.
PLAN.md's B1/B2/B3 hypotheses and stop rules are unchanged. The bootstrap now
asserts one actual NVIDIA display PCI function before installing packages or
copying the baseline. Physical die still requires unprivileged RM verification.
Baseline sourcef8485f6db, candidate36d134bc5. Existing nativeGB206Windows580
reference is reused only if this board measuresGB206; no neighboring die facts.
Full native-page injection remains unauthorized/not performed.

MEASURED19:29UTC: actual146GiB root/61logicalCPUs/202527MiBRAM/KVM,
PCI10de:2d04. Stock open595.91.07 works and reports8151MiB VRAM (do not infer
16GiB from marketing). UID65534/CapEff0 probe verifiesGB206 arch0x1b0/impl6.
Native gates9/9+gate10PASS19:27:50UTC,12/12USER births. Privatef8485f6d QEMU
build finished19:28:50UTC. Root595caps page saved as an oracle only. Thirteen
Windows chunks copied; full-image integrity check remains pending at this note.
Linux595raw/Windowson-off/candidate stillUNRUN; no unsupported-family fallback.

MEASURED19:33UTC: complete Windows image SHA matches539aaad2...044b,
Linux setup exits0 at19:29:44. Serial GPU controller91843started19:30:40,
native CUDA4 and Linux595raw30 are running. Windows on/off follows only after
raw teardown and no-QEMU check. Root/rental source remainsf8485f6db; no cargo
builds or source fetches during timed work. Structured display allocation export
uses compiler-derived public580.65.06 PIO/DMA fields and class IDs, redacts
pControl values, rejects serialized/truncated/unrelated allocations; no raw
allocation/Other4 bodies return. Four synthetic screening assertions pass.

MEASURED19:47UTC: full Linux595raw30/30PASS at180, normal6, sourcef8485f6db,
30actualenv{} receipts/146USER births/90CUDA thread postures.10rawHB0startup
warnings only in bus-info-sweep (82ms future-offset unsigned-wrap window), no
nonzero-HBformula warnings; PR23remainsdraft/readinessgapnamed. FiveXid31map
exactly to intentionaldefer-liveness/missing-page-fault arms. Complete proof in
second_attempt/{B3_RESULT.md,linux595/}. Windowsbindingphase1QGA/PScomplete,
drvbindrunning; no actualWindowscensus or displayon/off result yet.
