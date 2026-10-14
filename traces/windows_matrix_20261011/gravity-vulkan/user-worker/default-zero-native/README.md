# Native USER default-zero versus explicit-id1 mask fixture

STATUS: MEASURED, 2026-10-12. Probe source16a1b6a78a02a9280c6e50010e831b73d01beed7;
RTX4070/host595.91.07/uid65534/stock unprivileged USER/copyclassC7B5.
Falsifier and one-variable statement precede the run in campaign.log.

Default-zero and explicit-id1 bothPASS: initial marker2, all11 excludedmaskbits
ignore99, STORE/USE restore, held48samples/maxGET1/noearlyref, realcopy/semaphore/
reference1/finalGET=PUT4. Cleanup success/nativehealth/nohostXid. Same272byte
commandfixture/ring/USERprivilege/physical-CE prohibition; only native allocation
subDeviceId differs. Native modeEXIT0each and harnessEXIT0 mean the oracle passes.

This establishes host595.91.07 USER zero/id1 equivalence for this tested fixture.
It does not establish guestWindows580.88 privileged GSP initialization or a
per-driver/die normalization rule. No runtime zero->one behavior added. Source
review: public595.84 UVM/watchdog intentionallyleavezero; CPU/GSP forwarding copies
unchanged; actual defaultRAMFC initialization absent fromopenCPUtree.
