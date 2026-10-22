TURING RESUME AVAILABILITY — 2026-10-22

1. CODE READ (2026-10-22): requested Turing after Ampere, targeting TU106/TU104 consumer and mapped Quadro RTX workstation variants, one owned lease at a time.
2. MEASURED (2026-10-22, `turing_availability/no_offer_census.json`): no eligible offer at23:52:59Z; rental selector refused before creating a lease.
3. MEASURED (2026-10-22, `turing_availability/expanded_offer_census.json`): verified and unverified provider queries,180GB storage, Super/Ti and Quadro RTX naming variants returned0 KVM candidates at23:53:26Z.
4. MEASURED (2026-10-22, `turing_availability/availability_control_census.json`): diagnostic5GB-storage search returned28 Turing offers across consumer models; all28 advertised vms_enabled=false. These are not eligible180GB Windows quotes.
5. MEASURED (2026-10-22, same control census): KVM/single-GPU/180GB/price<=1 control query returned31 offers for other GPU families; API was returning marketplace data.
6. MEASURED (2026-10-22, `turing_availability/final_offer_census.json`): final23:57:27Z census returned30 KVM offers and0 Turing candidates; GTX16x names were included in this final check.
7. INFERRED (2026-10-22): Turing is unavailable under this run's KVM/storage/price requirements in these snapshots; this does not establish permanent unavailability or permission to rent non-KVM offers.
8. MEASURED (2026-10-22, `final_cleanup.json`): new Turing resume leases0, new Turing rental cost0; all earlier own lease IDs API-absent.
9. MEASURED (2026-10-22, `../lease_57262021/TEARDOWN.json`): earlier TU106 preparation rental was already destroyed22:02:29Z; no new native/engine/Windows acceptance probe ran there.
10. CODE READ (2026-10-22, `coverage.json`, constants audit): existing TU106 tables/captures are retained; unavailable acceptance testing is not a reason to repeat existing constant measurements.
11. CODE READ (2026-10-22, `coverage.json`): main target20 consumer/workstation dies; any-item constant receipts14, inherited0, unmeasured6 (TU104,TU117,GA103,GA107,AD107,GB207),30% of equal-weight die set.
12. CODE READ (2026-10-22, `TIER_PLAN.md`): workstation driver/vBIOS/ECC axes remain unverified; harvested masks/counts/routes are host runtime facts, never inherited die constants; datacenter remains separate/last.
13. CODE READ (2026-10-22): new Turing Windows580.88 bind/ON/desktop/appmatrix/Unigine/600s hold NOTRUN; hardware n0,spread unavailable; no synthetic PASS or failing RPC/event time claimed.
14. INFERRED (2026-10-22, `final_cleanup.json`): lane conservative total$6.722848<12 and Round3$2.919230<8; budget is not the blocker. Price guard$1/h and reserve$5 remain intact; invoice not verified.
15. INFERRED (2026-10-22): next required hardware action is Turing acceptance when a compliant offer is available; no RE is needed for the owner-approved CE-family admission, and no RE was done in S4.
