# traces/ index

**STATUS: LIVE, 2026-11-23 (generated once, not kept in sync).** One row per directory under `traces/` at `next` `92a7f2782`. Read it as a finding aid, not as evidence: the columns are the directory name, whether it has a `README.md`, whether its bulk files were moved to an archive (`MANIFEST.tsv`, see [`EVIDENCE_ARCHIVE.md`](EVIDENCE_ARCHIVE.md)), the number of tracked files, and the date of the commit that first added the directory (which is the date the evidence was committed, not necessarily the date of the run).

**Owner-visible gap:** 96 of 280 directories have no `README.md`, so no run, revision or date is recorded for them beside their name and the commit that added them. The rule is that a directory carries a README naming the run, the revision and the date; these do not yet. They were not guessed at here. Files read by tests or by the Vast box payload live under `traces/` (the `SPARSE` set in `ci/vast-broker/src/onstart.js` and the paths in `crates/*/data`); do not move or rename one without updating its reader.

The two replay traces at the top level of `traces/` and their provenance are described in [`README.md`](README.md).

| Directory | README | Bulk archived | Files | First committed |
|---|---|---|---|---|
| `adobe_vast_part2_20261122/` | yes |  | 23 | 2026-11-22 |
| `ampere_hw_20261103/` | yes | yes | 136 | 2026-11-03 |
| `antigravity_review_20261006/` | yes |  | 14 | 2026-10-06 |
| `apps_image_chroot_eval_20261108/` | yes |  | 6 | 2026-11-08 |
| `apps_image_inventory_split_20261108/` | yes |  | 11 | 2026-11-08 |
| `apps_image_recipe_20261107/` | yes |  | 17 | 2026-11-07 |
| `apps_image_rv2_20261108/` | yes |  | 19 | 2026-11-08 |
| `apps_linux_20261107/` | yes |  | 171 | 2026-11-07 |
| `apps_linux_media_20261110/` | yes |  | 5 | 2026-11-10 |
| `at1_20261018/` | yes |  | 1 | 2026-10-22 |
| `blackwell_ab_20261023/` | **no** |  | 74 | 2026-10-27 |
| `blackwell_adaptive_tier3_20261023/` | yes |  | 41 | 2026-10-23 |
| `blackwell_cached_validation_20261023/` | yes |  | 118 | 2026-10-23 |
| `blackwell_consttab_20261023/` | **no** |  | 1 | 2026-10-23 |
| `blackwell_edge_validation_20261023/` | yes |  | 75 | 2026-10-23 |
| `blackwell_fields_phase_20261022/` | yes |  | 80 | 2026-10-22 |
| `blackwell_host_caps_review_20261022/` | **no** |  | 28 | 2026-10-22 |
| `blackwell_hw_20261103/` | yes |  | 158 | 2026-11-03 |
| `blackwell_native_edge_probe_20261022/` | yes |  | 7 | 2026-10-22 |
| `blackwell_native_ref_20261018/` | **no** |  | 10 | 2026-10-27 |
| `blackwell_nvdec_admission_20261020/` | **no** |  | 8 | 2026-10-20 |
| `blackwell_provider_validation_20261023/` | yes |  | 93 | 2026-10-23 |
| `blackwell_resume_20261020/` | **no** |  | 3 | 2026-10-22 |
| `blackwell_resume_20261021/` | **no** | yes | 71 | 2026-10-21 |
| `blackwell_resume_cont_20261022/` | **no** | yes | 172 | 2026-10-22 |
| `blackwell_validation2_20261022/` | **no** |  | 38 | 2026-10-22 |
| `blackwell_validation3_20261022/` | yes |  | 6 | 2026-10-22 |
| `blackwell_validation4_20261022/` | **no** | yes | 121 | 2026-10-22 |
| `blackwell_validation_20261022/` | yes |  | 89 | 2026-10-22 |
| `blackwell_win_20261103/` | yes | yes | 152 | 2026-11-03 |
| `blackwell_windows_resume_20261023/` | yes |  | 131 | 2026-10-23 |
| `boots/` | **no** | yes | 379 | 2026-08-11 |
| `bw11_20261026/` | yes |  | 2 | 2026-10-27 |
| `c_oracle_census/` | **no** |  | 1 | 2026-08-01 |
| `candidate5_20261018/` | **no** |  | 16 | 2026-10-18 |
| `capability_535_545_audit_20260928/` | yes |  | 103 | 2026-09-28 |
| `caps_sweep_20261103/` | yes |  | 22 | 2026-11-03 |
| `cleanup_sweep_20261018/` | **no** |  | 1 | 2026-10-22 |
| `constants_measure_20261014b/` | **no** |  | 62 | 2026-10-22 |
| `constants_night_20261014/` | **no** |  | 5 | 2026-10-22 |
| `cudart_ga106/` | yes | yes | 3 | 2026-08-20 |
| `cursor_live_20261015/` | yes |  | 1 | 2026-10-15 |
| `cw1_constants_20261018/` | **no** |  | 3 | 2026-10-22 |
| `cw1_consttab_20261018/` | **no** |  | 5 | 2026-10-22 |
| `deferred_api_translated_20261008/` | **no** |  | 6 | 2026-10-08 |
| `descriptor_bound_20261018/` | **no** |  | 1 | 2026-11-09 |
| `diagnostic_gates_20261018/` | yes |  | 1 | 2026-10-18 |
| `die_coverage_20261104/` | **no** |  | 1201 | 2026-11-04 |
| `die_driver_sweep_20261015/` | **no** |  | 11 | 2026-10-19 |
| `die_sweep_20261014/` | **no** |  | 25 | 2026-10-22 |
| `display_reply_diff_20261008/` | yes | yes | 33 | 2026-10-09 |
| `doc_rot_20261018/` | **no** |  | 6 | 2026-10-23 |
| `doorbell_baseline_20260928/` | **no** |  | 1 | 2026-09-28 |
| `doorbell_contract_20261017/` | **no** |  | 1 | 2026-10-17 |
| `driver_matrix/` | **no** |  | 131 | 2026-09-26 |
| `ens8_20261027/` | yes |  | 6 | 2026-10-27 |
| `family_evidence_20261027/` | yes |  | 6 | 2026-10-27 |
| `flake_concurrent_fuzz_20261104/` | **no** |  | 3 | 2026-11-04 |
| `ga106_win_boot1_20261106/` | yes |  | 28 | 2026-11-06 |
| `gate_reuse_20261018/` | yes |  | 7 | 2026-10-28 |
| `gate_windows_ci_20261104/` | yes |  | 6 | 2026-11-04 |
| `gate_windows_ci_20261105/` | yes |  | 3 | 2026-11-05 |
| `gates/` | **no** |  | 16 | 2026-08-11 |
| `gl_linux_regression_20261011/` | yes |  | 34 | 2026-10-11 |
| `guest_boots/` | **no** |  | 542 | 2026-08-09 |
| `hitch_20261017/` | yes | yes | 7 | 2026-10-18 |
| `hostabi_595_rows_20261118/` | yes |  | 73 | 2026-11-19 |
| `hwrun_20261113/` | yes |  | 39 | 2026-11-13 |
| `hwval_20261120/` | yes |  | 143 | 2026-11-21 |
| `ignore_ladder_20261105/` | yes |  | 15 | 2026-11-05 |
| `imageb_apps_20261116/` | yes |  | 229 | 2026-11-16 |
| `imageb_gpu_step_20261117/` | yes |  | 65 | 2026-11-17 |
| `kf3_read_trace_20261009/` | yes | yes | 13 | 2026-10-09 |
| `kfci_pm1_20261108/` | **no** |  | 3 | 2026-11-08 |
| `kfci_pm1_rv2_20261108/` | **no** |  | 3 | 2026-11-08 |
| `kfml_run_20261116/` | yes |  | 30 | 2026-11-16 |
| `kfml_run_20261117_second_die/` | yes |  | 28 | 2026-11-17 |
| `kfml_run_20261117_tier_decision/` | yes |  | 77 | 2026-11-17 |
| `landing_trim_20261028/` | yes |  | 3 | 2026-10-28 |
| `lazy_prod_20261017/` | **no** |  | 1 | 2026-10-17 |
| `lifecycle_review_20261016/` | **no** |  | 1 | 2026-10-16 |
| `linux_color_20261006/` | yes | yes | 22 | 2026-10-06 |
| `linux_planes_oracle_20261014/` | yes | yes | 85 | 2026-10-14 |
| `linux_regression_20261010/` | yes | yes | 61 | 2026-10-11 |
| `linux_s3_resume_20261014/` | yes |  | 14 | 2026-10-14 |
| `lke193_phase0_20261109/` | yes |  | 12 | 2026-11-09 |
| `llm_parity/` | **no** | yes | 29 | 2026-09-25 |
| `lut_tokens_20261018/` | yes |  | 1 | 2026-10-18 |
| `matlab_gpu_20261117/` | yes |  | 14 | 2026-11-17 |
| `merge_bar_20261008/` | yes |  | 8 | 2026-10-08 |
| `middle_real_apps_20261109/` | **no** |  | 11 | 2026-11-09 |
| `n1_control_20261121/` | yes |  | 29 | 2026-11-21 |
| `nvdiff_w292/` | yes | yes | 5 | 2026-08-14 |
| `open_ml_rows_20261117/` | yes |  | 35 | 2026-11-17 |
| `open_rows_ffdcgm_20261117/` | yes |  | 24 | 2026-11-17 |
| `opengl_gears_20261011/` | yes | yes | 41 | 2026-10-11 |
| `parallel_fetch_20261107/` | **no** |  | 1 | 2026-11-07 |
| `parked_2026_08_15/` | **no** |  | 1 | 2026-08-15 |
| `passthrough_interrupt_20261008/` | **no** |  | 14 | 2026-10-08 |
| `passthrough_nsi_nogate_20261008/` | **no** |  | 46 | 2026-10-08 |
| `perf_baseline_6692e621_20261010/` | **no** |  | 194 | 2026-10-10 |
| `perf_status_20261017/` | yes |  | 1 | 2026-10-17 |
| `phys_operand_oracle_20261008/` | yes |  | 1 | 2026-10-08 |
| `postcomp_units_20261021/` | **no** |  | 12 | 2026-10-21 |
| `pr424_hardware_20261115/` | yes |  | 134 | 2026-11-15 |
| `propimage_linux_slice1_20261116/` | yes |  | 20 | 2026-11-16 |
| `rawclient_all_drivers_20261008/` | **no** |  | 3 | 2026-10-08 |
| `rawclient_ce_interrupt_20261008/` | **no** |  | 15 | 2026-10-08 |
| `real_ad104/` | yes |  | 2 | 2026-09-26 |
| `real_ga104/` | yes |  | 2 | 2026-09-26 |
| `real_ga106/` | yes |  | 35 | 2026-08-01 |
| `real_hw_20261013/` | **no** |  | 1 | 2026-10-22 |
| `real_tu116/` | yes |  | 2 | 2026-09-28 |
| `real_windows_20261014/` | yes |  | 60 | 2026-10-14 |
| `recovered_display_20260929/` | yes |  | 14 | 2026-09-29 |
| `recovered_integration_20260929/` | yes | yes | 99 | 2026-09-29 |
| `recovery_20260928/` | yes |  | 3 | 2026-09-30 |
| `release_checks_20261107/` | yes |  | 10 | 2026-11-07 |
| `release_manual_rehearsal_20261115/` | yes |  | 65 | 2026-11-15 |
| `resolve_gpu_20261116/` | yes |  | 19 | 2026-11-16 |
| `runlog_20261105/` | yes |  | 3 | 2026-11-05 |
| `sa01_20261018/` | yes |  | 1 | 2026-10-22 |
| `sa04_20261018/` | yes |  | 1 | 2026-10-22 |
| `sa06_20261018/` | yes |  | 1 | 2026-10-22 |
| `sdr_lut_20261006/` | yes |  | 14 | 2026-10-06 |
| `secfix_gpu_20261113/` | yes |  | 19 | 2026-11-13 |
| `single_store_crossing/` | yes |  | 4 | 2026-09-15 |
| `single_store_e1/` | yes |  | 11 | 2026-09-14 |
| `single_store_e4/` | yes |  | 9 | 2026-09-14 |
| `single_store_w724g/` | yes |  | 6 | 2026-09-14 |
| `site1_20261018/` | yes |  | 2 | 2026-10-26 |
| `site_owner_batch_20261028/` | yes |  | 2 | 2026-10-28 |
| `site_status_20261018/` | yes |  | 5 | 2026-10-28 |
| `spec_stage_proof_20261109/` | yes |  | 31 | 2026-11-10 |
| `specviewperf_vast_20261105/` | yes |  | 38 | 2026-11-05 |
| `standalone_wave1_20261119/` | yes |  | 696 | 2026-11-20 |
| `standalone_wave1b_linux_20261121/` | yes |  | 111 | 2026-11-21 |
| `standalone_wave1b_windows_20261121/` | yes |  | 356 | 2026-11-21 |
| `sw3_die_sweep_20261024/` | yes |  | 54 | 2026-10-24 |
| `sw5_die_sweep_20261025/` | yes |  | 55 | 2026-10-27 |
| `sweep_ada_20261018/` | **no** |  | 32 | 2026-10-23 |
| `sweep_continue_20261020/` | **no** |  | 49 | 2026-10-20 |
| `sweep_dies_20261018/` | **no** |  | 30 | 2026-10-23 |
| `sweep_integrate_20261018/` | **no** | yes | 600 | 2026-10-18 |
| `sweep_windows_20261021/` | **no** |  | 174 | 2026-10-21 |
| `sz1_software_wishlist_20261107/` | yes |  | 4 | 2026-11-07 |
| `tmo_stage_20261006/` | yes | yes | 45 | 2026-10-06 |
| `translated_rc_20261018/` | yes |  | 16 | 2026-10-22 |
| `trio_vast_20261105/` | yes |  | 99 | 2026-11-05 |
| `tur1_20261027/` | yes |  | 71 | 2026-10-27 |
| `turing_boot_20261103/` | yes | yes | 102 | 2026-11-03 |
| `user_ring_scan_20261014/` | **no** |  | 23 | 2026-10-14 |
| `util_layout_20261117/` | yes |  | 21 | 2026-11-17 |
| `v3_adasys/` | **no** |  | 40 | 2026-09-26 |
| `v3_app_matrix/` | **no** |  | 203 | 2026-09-26 |
| `v3_appfix/` | yes |  | 18 | 2026-09-26 |
| `v3_blackwell/` | **no** |  | 10 | 2026-09-26 |
| `v3_candidates/` | **no** | yes | 1802 | 2026-10-04 |
| `v3_cdp/` | yes | yes | 91 | 2026-09-30 |
| `v3_ci/` | yes | yes | 3 | 2026-09-30 |
| `v3_cifix/` | yes | yes | 3 | 2026-09-30 |
| `v3_design_review_20261003/` | yes | yes | 3 | 2026-10-03 |
| `v3_display/` | **no** | yes | 1227 | 2026-09-27 |
| `v3_families_master/` | yes |  | 3 | 2026-09-30 |
| `v3_gfx/` | **no** | yes | 9 | 2026-09-26 |
| `v3_gfxset/` | **no** | yes | 740 | 2026-09-26 |
| `v3_gpcmask/` | yes |  | 13 | 2026-09-26 |
| `v3_initrace/` | yes | yes | 15 | 2026-09-26 |
| `v3_int_ga106/` | **no** |  | 7 | 2026-09-26 |
| `v3_ioeventfd/` | yes | yes | 29 | 2026-09-30 |
| `v3_mc20/` | yes |  | 8 | 2026-09-27 |
| `v3_mc21/` | yes |  | 6 | 2026-09-27 |
| `v3_mc22/` | yes | yes | 32 | 2026-09-30 |
| `v3_mc23/` | yes |  | 2 | 2026-09-30 |
| `v3_ramobj/` | yes |  | 2 | 2026-09-30 |
| `v3_refusal_audit/` | **no** | yes | 106 | 2026-09-26 |
| `v3_roperm/` | yes |  | 55 | 2026-09-26 |
| `v3_scratch_bound/` | **no** |  | 2 | 2026-10-03 |
| `v3_security/` | **no** |  | 34 | 2026-10-03 |
| `v3_turing/` | **no** |  | 48 | 2026-09-28 |
| `v3_turing_master/` | yes |  | 2 | 2026-09-30 |
| `v3_uvm_b3/` | yes |  | 7 | 2026-09-30 |
| `v3_uvm_research/` | yes |  | 7 | 2026-09-26 |
| `v3_windows/` | **no** | yes | 117 | 2026-10-04 |
| `vanish_hunt_20261106/` | yes | yes | 14 | 2026-11-06 |
| `vast_admin_20261018/` | yes |  | 7 | 2026-10-26 |
| `vast_admin_v2_20261018/` | yes |  | 8 | ? |
| `vast_auth401_20261018/` | yes |  | 7 | 2026-10-27 |
| `vast_badhost_20261018/` | yes |  | 6 | 2026-10-27 |
| `vast_broker_summary/` | yes |  | 2 | 2026-10-23 |
| `vast_cloud_init_20261018/` | yes |  | 15 | 2026-10-23 |
| `vast_payload_v7_20261018/` | yes |  | 26 | 2026-10-23 |
| `vast_setup_v8_20261018/` | **no** |  | 17 | 2026-10-23 |
| `vast_setup_v8_fresh_20261018/` | **no** |  | 10 | 2026-10-23 |
| `vast_xid_followup_20261018/` | yes |  | 16 | 2026-10-23 |
| `vast_xid_v9_20261018/` | yes |  | 28 | 2026-10-23 |
| `vh_archive/` | yes | yes | 4 | 2026-09-26 |
| `video_ga106/` | **no** |  | 11 | 2026-09-26 |
| `vkcts_host_20261117/` | yes |  | 8 | 2026-11-17 |
| `w294_cudalimit/` | yes | yes | 11 | 2026-08-14 |
| `w297_cup3/` | yes |  | 6 | 2026-08-14 |
| `w298_ablation/` | yes | yes | 46 | 2026-08-14 |
| `w299_multiproc/` | **no** | yes | 19 | 2026-08-14 |
| `w304_confirm/` | yes |  | 33 | 2026-08-14 |
| `w305_preempt_and_crit1/` | **no** |  | 9 | 2026-08-14 |
| `w308_cup8/` | **no** | yes | 12 | 2026-08-14 |
| `w309_crit1/` | **no** |  | 19 | 2026-08-14 |
| `w313_restore/` | **no** |  | 22 | 2026-08-14 |
| `w314_confirm/` | yes | yes | 35 | 2026-08-14 |
| `w315_floor/` | yes | yes | 36 | 2026-08-14 |
| `w317_drain/` | yes | yes | 53 | 2026-08-14 |
| `w318_gate/` | yes | yes | 35 | 2026-08-14 |
| `w319_intermittent/` | **no** | yes | 49 | 2026-08-14 |
| `w320_sync/` | yes | yes | 15 | 2026-08-14 |
| `w321_batch/` | **no** | yes | 58 | 2026-08-14 |
| `w322_operands/` | yes | yes | 31 | 2026-08-14 |
| `w326_publish/` | yes |  | 29 | 2026-08-14 |
| `w327_cliff/` | yes | yes | 3 | 2026-08-14 |
| `w328_scope/` | yes |  | 116 | 2026-08-15 |
| `w329_release/` | yes | yes | 3 | 2026-08-15 |
| `w388_defer_liveness/` | **no** |  | 3 | 2026-09-07 |
| `w390_invalidate_blockage/` | **no** |  | 53 | 2026-09-08 |
| `w392d_guest_wall/` | **no** |  | 1 | 2026-09-08 |
| `w394_cuda_apps/` | **no** |  | 6 | 2026-09-09 |
| `w734_fbio_census/` | yes |  | 7 | 2026-09-15 |
| `w735_guest_suite/` | yes |  | 8 | 2026-09-15 |
| `w736_fbstore/` | **no** |  | 3 | 2026-09-15 |
| `w738_fbstore_cutb/` | **no** |  | 2 | 2026-09-15 |
| `w739_fbstore_cutc/` | **no** |  | 2 | 2026-09-15 |
| `w740_scrub_arming/` | **no** |  | 2 | 2026-09-15 |
| `w742_publish_install/` | **no** |  | 2 | 2026-09-15 |
| `w743_preflight/` | **no** |  | 2 | 2026-09-15 |
| `w744_b1d_probe/` | **no** |  | 4 | 2026-09-15 |
| `w745_split/` | **no** |  | 3 | 2026-09-15 |
| `w746_handover/` | **no** |  | 3 | 2026-09-15 |
| `w747_list_object_alias/` | **no** |  | 3 | 2026-09-16 |
| `w750_route_k/` | **no** |  | 5 | 2026-09-16 |
| `w752_inplace_repoint/` | **no** | yes | 3 | 2026-09-16 |
| `w753_route_k/` | **no** | yes | 3 | 2026-09-16 |
| `w754_gsp_queue_off_vcpu/` | yes | yes | 12 | 2026-09-16 |
| `w786_suite/` | **no** |  | 30 | 2026-09-19 |
| `w825_overnight/` | **no** |  | 8 | 2026-09-24 |
| `walk_shadow/` | yes |  | 3 | 2026-09-15 |
| `walk_shadow_live/` | yes |  | 8 | 2026-09-15 |
| `walk_swap_live/` | yes |  | 6 | 2026-09-15 |
| `walk_wake_20261103/` | yes | yes | 51 | 2026-11-03 |
| `wb1_public_bundle_20261108/` | yes |  | 8 | 2026-11-08 |
| `wb1_republish_20261120/` | yes |  | 4 | 2026-11-20 |
| `wb1_republish_license_20261120/` | yes |  | 4 | 2026-11-20 |
| `wb1_republish_notices_20261121/` | yes |  | 4 | 2026-11-21 |
| `wc1_20261026/` | yes |  | 9 | 2026-10-27 |
| `win11_opengl_20261014/` | yes |  | 18 | 2026-10-14 |
| `win11_s3_20261014/` | yes |  | 18 | 2026-10-14 |
| `win11_userring_20261014/` | yes | yes | 17 | 2026-10-14 |
| `winapps_20261107/` | yes |  | 464 | 2026-11-07 |
| `windows_baseline_20261010/` | yes | yes | 313 | 2026-10-10 |
| `windows_capture_validation_20261013/` | **no** |  | 14 | 2026-10-13 |
| `windows_code43_walls_20261007/` | yes | yes | 614 | 2026-10-07 |
| `windows_exact_decode_20261007/` | yes | yes | 19 | 2026-10-07 |
| `windows_flag_ablation_20261010/` | yes |  | 44 | 2026-10-11 |
| `windows_flip_vsync_20261008/` | yes |  | 23 | 2026-10-08 |
| `windows_gate_20261101/` | yes | yes | 161 | 2026-11-01 |
| `windows_gl_crash_20261011/` | yes |  | 1 | 2026-10-11 |
| `windows_matrix_20261011/` | yes | yes | 2851 | 2026-10-11 |
| `windows_overlay_native_20261010/` | yes |  | 48 | 2026-10-10 |
| `windows_playback_tdr_20261010/` | yes | yes | 4 | 2026-10-10 |
| `windows_pool_20261005/` | yes |  | 26 | 2026-10-05 |
| `windows_prod_20261010/` | yes |  | 2 | 2026-10-10 |
| `windows_reset_20261009/` | yes | yes | 47 | 2026-10-09 |
| `windows_timer_20261005/` | **no** |  | 91 | 2026-10-05 |
| `wk1_budget_20261018/` | yes |  | 3 | 2026-10-26 |
| `wk1_domain_20261018/` | yes |  | 3 | 2026-10-26 |
| `wk1_green_count_20261018/` | yes |  | 3 | 2026-10-27 |
| `wk1_green_provider_20261018/` | yes |  | 4 | 2026-10-27 |
| `wk1_green_selection_20261018/` | yes |  | 3 | 2026-10-27 |
| `wk1_ids_20261018/` | yes |  | 6 | 2026-10-26 |
| `wk1_ledger_20261018/` | yes |  | 4 | 2026-10-26 |
| `wk1_silent_20261018/` | yes |  | 4 | 2026-10-26 |
| `wk2_resume_20261018/` | yes |  | 8 | 2026-10-28 |
| `worker_placement_20261018/` | **no** |  | 1 | 2026-10-25 |
