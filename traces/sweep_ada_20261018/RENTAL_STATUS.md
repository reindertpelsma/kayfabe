STATUS: LIVE, 2026-10-22. Guard verified and pushed; rental not started.

MEASURED: 97 software tests pass, including the unchanged original 23; 22 mutants
have at least two distinct killers each. Runtime/guard source is frozen at
35ff2820e5573199072ab2d274f5f0b91a1826a8, verified on origin.
MEASURED: no eligible AD106/AD104/AD103 verified single-GPU KVM offer is currently
listed. The complete verified VM search returned 17 offers; an additional search
including external verified offers also returned no listed die. Details are in
rental_availability.json. No lease has been created; rental spend is zero.
CODE READ: S2 specifies AD106 first, then AD104/AD103. Available AD102 is outside
that fallback list; the owner has been asked to choose explicitly before rental.
MEASURED: the candidate AD102 offer is 0.49 USD/h with 180GB. At 170 minutes,
maximum runtime is 1.388334 USD; adding a 1 USD delayed-billing guard remains below
this lane's 4 USD cap. The guard is not a claim of spend. Shared credit observed
43.531724 USD; at least 5 USD reserve is required at creation.
CODE READ: existing source-pinned native, Windows media/install/bind and arm
recipes have been prepared locally with the new guest supervisor; templates are
not deployed and still contain an explicit own-ID placeholder. The observer
option exists in the sweep harness; availability in the newly built QEMU would
be checked before its separate hold. No hardware PASS or observer result follows.
