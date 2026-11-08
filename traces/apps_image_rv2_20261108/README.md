STATUS: DESIGN-ONLY, 2026-11-08. CODE READ: RV2 review fixes and GPU-free controls; no real image build, VM, chroot or GPU qualification.

# LB1: observed verbatim input binding

**CODE READ:** closes both aspects of RV2 finding P2 against PR #304 at `b8f0ebf731adcbfcac29d5c1dd9d17601f138bcd`: local-source runtime copies are bound to their source SHA256, and deterministic model/configuration selectors are explicitly collected and content-pinned. Compiled binary, installed-distribution and image hashes remain informational.

**MEASURED, 2026-11-08:** tests-first [unit baseline](tests-first-red.txt) has 12 assertion failures and two missing-new-helper API errors across 14 controls; [fake-QEMU/collector baseline](fake-qemu-red.txt) has nine assertion failures across nine controls. These are red-baseline receipts, not mutant kills.

**MEASURED, 2026-11-08:** `origin/next` at `e9408ce4bf3f7956c74309d7b49e4b3e3230479d` was already an ancestor of the reviewed head. The requested merge returned “Already up to date” before these edits. No history was rewritten.

**CODE READ:** recipe `2026-11-08.2` derives runtime destination pins from exact/glob local-source copy declarations and retained local files. Five Python scripts and the retained compatibility runner are observed PINNED bytes; compile-only sources removed from `/opt/apps/source` are excluded. Builder acceptance checks bundle copies before the existing export audit, and assembly/verify validate them again. Recipe `write_files` and runtime `official_write_files` use SHA256 of exact UTF-8 content. Explicit collector paths cover `refs/main` outside inventory trees. No compiled output hash or installed-distribution digest became a comparison requirement.

**MEASURED, 2026-11-08 ([input audit](input-audit.json)):** all 19 local source pins match the checkout and both requirements locks match the recipe. Direct download pins and NVIDIA retention policy are unchanged; this review-fix session downloaded no vendor media.

**MEASURED, 2026-11-08 ([exact CI receipt](exact-ci.json)):** these exact Linux-image workflow commands returned zero:

```text
python3 scripts/ci/test_apps_image.py
python3 scripts/ci/test_apps_image_layers.py
python3 scripts/apps/image/mutants.py --jobs 2
```

| Surface | Reviewed head | After fixes |
|---|---:|---:|
| Core image tests | 153 | [181 pass](tests-core.txt) |
| Layer tests | 127 | [127 pass](tests-layers.txt) |
| Legacy payload tests | 64 | [64 pass](tests-legacy.txt) |
| Matrix table tests | 12 | [12 pass](tests-matrix.txt) |
| Selected total | 356 | 384 pass |
| Image mutants | 70 | 77/77 valid kills |

**MEASURED, 2026-11-08:** separate negative fake-QEMU controls reject changed `hf_generate.py` and `ai_bench.py` at build and verify, changed/missing model selectors at verify and missing selectors at build. Collector controls hash configuration outside app trees and reject missing files. The positive control `FakeQemu.test_verbatim_inputs_preserve_variable_compiled_and_image_bytes` succeeds with unchanged input pins and different compiled/image bytes. No real guest transcript is represented by these mocks.

**MEASURED, 2026-11-08:** [exact CI mutant log](mutants-ci.txt) and a separate [structured-receipt run](mutants-report-run.json) both pass 77/77 mutants. The additional run records [every named killer](mutants.json); it also rechecks both complete suite baselines. All kills below have at least two distinct assertion failures and zero test errors. Seven new mutations precede the previous 70 rules.

| Mutated rule | Assertion killers | Two named tests |
|---|---:|---|
| runtime-installer-configuration | 2 | `core:VerbatimInputs.test_changed_runtime_installer_config_is_rejected`; `core:VerbatimInputs.test_runtime_installer_config_is_pinned` |
| copied-source-pin-binding | 3 | `core:VerbatimInputs.test_changed_ai_bench_is_rejected`; `core:VerbatimInputs.test_changed_hf_generate_is_rejected` |
| configuration-pin-binding | 3 | `core:VerbatimInputs.test_changed_model_ref_is_rejected`; `core:VerbatimInputs.test_changed_runtime_installer_config_is_rejected` |
| observed-runtime-inputs-pinned | 6 | `core:VerbatimInputs.test_ai_bench_observed_bytes_are_pinned`; `core:VerbatimInputs.test_direct_retained_local_source_is_pinned` |
| collect-runtime-configuration | 3 | `core:RuntimeInputCollector.test_missing_model_ref_is_a_collection_failure`; `core:RuntimeInputCollector.test_model_ref_outside_inventory_trees_is_hashed` |
| builder-copied-source-binding | 2 | `core:VerbatimInputs.test_builder_rejects_ai_bench_copy_drift`; `core:VerbatimInputs.test_builder_rejects_hf_generate_copy_drift` |
| conflicting-runtime-pins | 2 | `core:VerbatimInputs.test_conflicting_artifact_and_config_pins_rejected`; `core:VerbatimInputs.test_conflicting_copy_and_config_pins_rejected` |
| built-part-not-compared | 5 | `core:Inventory.test_verify_allows_different_binary_bytes`; `core:Inventory.test_verify_allows_different_compiled_app_bytes` |
| input-pins-visible | 4 | `core:Inventory.test_pinned_records_model_input_url_and_hash`; `core:Inventory.test_pinned_records_wheel_input_url_and_hash` |
| recipe-hash-visible | 2 | `core:Inventory.test_recipe_environment_drift_changes_pinned_identity`; `core:Inventory.test_recipe_version_drift_changes_pinned_identity` |
| source-refs-visible | 2 | `core:Inventory.test_source_commit_is_visible_in_pinned_part`; `core:Inventory.test_source_tag_is_visible_in_pinned_part` |
| missing-app-comparison | 2 | `core:Inventory.test_compare_refuses_missing_compiled_app`; `core:Inventory.test_compare_refuses_missing_packaged_app` |
| packaged-app-presence | 2 | `core:Inventory.test_inventory_refuses_another_missing_packaged_app`; `core:Inventory.test_inventory_refuses_missing_packaged_app` |
| manifest-input-consistency | 2 | `core:FakeQemu.test_manifest_cannot_substitute_recorded_source_hash`; `core:FakeQemu.test_manifest_cannot_substitute_recorded_source_url` |
| manifest-required-apps | 2 | `core:FakeQemu.test_manifest_cannot_omit_compiled_app_presence`; `core:FakeQemu.test_manifest_cannot_omit_packaged_app_presence` |
| guest-compiled-hash-is-advisory | 2 | `core:GuestInstall.test_guest_records_first_compiled_output_without_expected_hash_gate`; `core:GuestInstall.test_guest_records_second_compiled_output_without_expected_hash_gate` |
| missing-input-hash | 2 | `core:Pins.test_missing_base_hash_refused`; `core:Pins.test_missing_model_hash_refused` |
| public-url-check | 2 | `core:Pins.test_no_http_base`; `core:Pins.test_no_url_credentials` |
| snapshot-check | 2 | `core:Pins.test_malformed_snapshot_refused`; `core:Pins.test_moving_snapshot_refused` |
| shell-wrapper-check | 2 | `core:Pins.test_shell_absolute_path_refused`; `core:Pins.test_shell_command_refused` |
| download-digest-check | 2 | `core:Downloads.test_corrupt_download_refused`; `core:Downloads.test_poisoned_cache_refused` |
| download-size-check | 2 | `core:Downloads.test_cached_length_must_match_even_with_correct_hash`; `core:Downloads.test_declared_length_must_match_even_with_correct_hash` |
| cache-reverification | 3 | `core:Downloads.test_cached_length_must_match_even_with_correct_hash`; `core:Downloads.test_oversize_cache_refused` |
| archive-path-check | 4 | `core:Archives.test_absolute_member_rejected_before_extraction`; `core:Archives.test_absolute_tar_path_refused` |
| guest-identity-check | 2 | `core:Inventory.test_wrong_guest_kernel_refused`; `core:Inventory.test_wrong_guest_module_version_refused` |
| apt-inventory-completeness | 2 | `core:Inventory.test_missing_apt_package_refused`; `core:Inventory.test_missing_other_apt_package_refused` |
| pip-files-completeness | 2 | `core:Inventory.test_pip_distribution_digest_extra_refused`; `core:Inventory.test_pip_distribution_digest_missing_refused` |
| model-inventory-pin | 2 | `core:Inventory.test_corrupt_model_refused`; `core:Inventory.test_model_config_digest_must_match_its_own_pin` |
| compiled-hash-is-advisory | 2 | `core:Inventory.test_expected_binary_hash_is_not_a_gate`; `core:Inventory.test_pinned_binary_hash_is_informational` |
| pip-freeze-check | 2 | `core:FakeQemu.test_build_refuses_pip_version_drift`; `core:Inventory.test_pip_drift_refused` |
| open-nvidia-boundary | 2 | `layers:LayerRecipe.test_nvidia_open_artifact_is_rejected`; `layers:LayerRecipe.test_nvidia_pip_wheel_cannot_be_moved_into_open_layer` |
| open-apt-components | 3 | `layers:LayerRecipe.test_multiverse_recipe_is_rejected`; `layers:LayerRecipe.test_restricted_or_multiverse_recipe_is_rejected` |
| open-scan-required | 5 | `layers:LayerInventories.test_clean_flag_cannot_hide_a_reported_proprietary_header`; `layers:LayerInventories.test_clean_flag_cannot_hide_a_reported_vendor_file` |
| official-receipts-required | 2 | `layers:LayerInventories.test_runtime_needs_actual_official_install_receipts`; `layers:LayerInventories.test_runtime_receipt_hash_cannot_drift` |
| inventory-comparison | 6 | `core:Inventory.test_recipe_environment_drift_changes_pinned_identity`; `core:Inventory.test_recipe_version_drift_changes_pinned_identity` |
| completion-marker | 3 | `core:FakeQemu.test_duplicate_completion_marker_refused`; `core:FakeQemu.test_marker_with_wrong_inventory_digest_refused` |
| fresh-session-check | 2 | `core:FakeQemu.test_stale_completion_refused`; `core:FakeQemu.test_verify_rejects_stale_guest_session` |
| qemu-exit-status | 2 | `core:FakeQemu.test_nonzero_qemu_exit_refused_despite_complete_report`; `core:FakeQemu.test_verify_rejects_nonzero_qemu_exit` |
| qemu-img-exit-status | 2 | `core:FakeQemu.test_qemu_img_check_failure_does_not_publish_manifest`; `core:FakeQemu.test_qemu_img_failure_does_not_publish_manifest` |
| assembled-export-integrity | 2 | `core:FakeQemu.test_assembled_app_must_match_the_audited_builder_bytes`; `core:FakeQemu.test_assembled_image_cannot_drop_upstream_licence_files` |
| guest-pip-hash-check | 2 | `core:GuestInstall.test_guest_pip_installs_only_hashed_offline_wheels`; `core:GuestInstall.test_guest_wheel_resolution_enforces_hashes_without_public_index` |
| guest-pip-index-check | 2 | `core:GuestInstall.test_guest_pip_installs_only_hashed_offline_wheels`; `core:GuestInstall.test_guest_wheel_resolution_enforces_hashes_without_public_index` |
| guest-command-status | 2 | `core:GuestInstall.test_guest_actual_nonzero_command_refused`; `core:GuestInstall.test_guest_subprocess_is_argument_list_without_shell` |
| builder-header-link-boundary | 2 | `layers:BuilderBoundary.test_builder_does_not_export_a_cuda_library_directory_link`; `layers:BuilderBoundary.test_builder_does_not_export_proprietary_cuda_headers_via_symlink` |
| opencl-gpu-selection | 2 | `layers:OpenClTarget.test_all_other_icds_are_disabled_before_the_gpu_gate`; `layers:OpenClTarget.test_provisioning_disables_pocl_registration` |
| opencl-nvidia-required | 2 | `layers:OpenClTarget.test_empty_icd_directory_cannot_be_a_provisioned_gpu_runtime`; `layers:OpenClTarget.test_provisioning_cannot_accept_a_cpu_only_opencl_platform` |
| proprietary-header-scan | 5 | `layers:BundledCudaHeaders.test_proprietary_cuda_header_cannot_hide_behind_bsd_marker`; `layers:BundledCudaHeaders.test_wrapped_proprietary_confidential_header_fails` |
| vendor-payload-path-scan | 7 | `layers:Scan.test_cuda_runtime_file_fails_without_header`; `layers:Scan.test_nvidia_python_distribution_fails` |
| vendor-package-scan | 2 | `layers:Scan.test_nvidia_deb_origin_fails_even_without_license_text`; `layers:StrictNoticeScan.test_nvidia_cuda_deb_is_not_allowed_without_files` |
| unknown-notice-scan | 3 | `layers:KernelOssHeaders.test_kernel_source_location_does_not_waive_unknown_nvidia_notices`; `layers:StrictNoticeScan.test_nvidia_copyright_notice_is_not_silently_waived` |
| official-http-status | 2 | `layers:OfficialFetch.test_monthly_failure_cannot_be_a_success_report`; `layers:OfficialFetch.test_unsuccessful_head_status_cannot_be_healthy` |
| official-content-length | 2 | `layers:OfficialFetch.test_monthly_size_drift_is_not_success`; `layers:OfficialFetch.test_wrong_length_is_media_drift` |
| monthly-failure-status | 3 | `layers:AdditionalLayerFailures.test_monthly_checks_continue_after_a_missing_url`; `layers:OfficialFetch.test_monthly_failure_cannot_be_a_success_report` |
| guest-output-byte-cap | 2 | `layers:BoundedTransport.test_inventory_sink_rejects_bytes_over_its_limit`; `layers:BoundedTransport.test_serial_sink_rejects_bytes_over_its_limit` |
| shared-runtime-dependency | 2 | `layers:ElfLinking.test_missing_shared_cublas_is_rejected`; `layers:ElfLinking.test_missing_shared_cudart_is_rejected` |
| apt-mirror-components | 2 | `layers:FrozenApt.test_multiverse_mirror_is_refused`; `layers:FrozenApt.test_restricted_mirror_is_refused` |
| apt-pool-boundary | 2 | `layers:FrozenApt.test_main_universe_index_cannot_redirect_to_multiverse_pool`; `layers:FrozenApt.test_main_universe_index_cannot_redirect_to_restricted_pool` |
| apt-download-budget | 2 | `layers:FrozenApt.test_apt_download_budget_fails_closed`; `layers:FrozenApt.test_zero_apt_budget_downloads_nothing` |
| export-file-digests | 2 | `layers:ExportTransfer.test_corrupt_export_file_fails_before_final_guest`; `layers:ExportTransfer.test_export_reported_hash_must_match_transferred_bytes` |
| export-cuda-path-boundary | 2 | `layers:ExportTransfer.test_cuda_header_tree_cannot_be_exported`; `layers:ExportTransfer.test_cuda_library_tree_cannot_be_exported` |
| export-special-file-boundary | 2 | `layers:ExportTransfer.test_device_file_cannot_be_exported`; `layers:ExportTransfer.test_fifo_cannot_be_exported` |
| export-static-output-boundary | 2 | `layers:ExportTransfer.test_static_device_fatbin_cannot_enter_open_export`; `layers:ExportTransfer.test_static_runtime_output_cannot_be_exported` |
| seed-required-files | 2 | `core:Seed.test_seed_rejects_extra_files`; `core:Seed.test_seed_rejects_incomplete_nocloud_data` |
| seed-volume-label | 2 | `core:Seed.test_both_iso_namespaces_advertise_cidata`; `core:Seed.test_seed_primary_volume_descriptor` |
| forward-endpoint-validation | 2 | `core:ForwardedInputs.test_input_forward_refuses_an_external_host`; `core:ForwardedInputs.test_input_forward_refuses_credentials` |
| explicit-input-forward | 2 | `core:ForwardedInputs.test_builder_has_explicit_forward_to_pinned_input_server`; `core:ForwardedInputs.test_provisioning_can_only_forward_to_local_official_cache` |
| scan-traversal-errors | 2 | `layers:ScanTraversalErrors.test_io_error_during_traversal_cannot_certify_a_clean_image`; `layers:ScanTraversalErrors.test_permission_denied_directory_cannot_certify_a_clean_image` |
| scan-resource-bounds | 2 | `layers:AdditionalLayerFailures.test_scan_byte_budget_exhaustion_is_failure`; `layers:AdditionalLayerFailures.test_scan_file_budget_exhaustion_is_failure` |
| official-disk-writer-digest | 2 | `layers:OfficialInputDisk.test_writer_refuses_changed_official_bytes`; `layers:OfficialInputDisk.test_writer_refuses_corrupt_second_payload` |
| official-disk-reader-digest | 2 | `layers:OfficialInputDisk.test_reader_rejects_corrupt_guest_visible_payload`; `layers:OfficialInputDisk.test_reader_rejects_payload_with_correct_length_and_wrong_digest` |
| official-disk-declared-size | 2 | `layers:OfficialInputDisk.test_device_index_cannot_inflate_declared_size`; `layers:OfficialInputDisk.test_device_index_cannot_shrink_declared_size` |
| official-disk-existing-file | 2 | `layers:OfficialInputDisk.test_existing_disk_is_never_overwritten_or_unlinked`; `layers:OfficialInputDisk.test_existing_symlink_is_not_followed_or_deleted` |
| official-guest-device-required | 2 | `layers:GuestOfficialTransport.test_another_block_device_cannot_be_read_as_official_media`; `layers:GuestOfficialTransport.test_missing_official_device_cannot_fall_back_to_network` |
| official-disk-readonly | 2 | `core:FakeQemu.test_builder_input_disk_is_readonly_and_not_attached_to_open_assembly`; `core:ForwardedInputs.test_provisioning_official_input_disk_is_explicitly_readonly` |
| official-http-isolation | 2 | `core:FakeQemu.test_open_assembly_http_cannot_serve_nvidia_payloads`; `core:FakeQemu.test_scratch_builder_http_cannot_serve_nvidia_payloads` |
| kernel-source-classification | 2 | `layers:KernelOssHeaders.test_gpl_header_named_nvidia_is_open_source`; `layers:KernelOssHeaders.test_nvidia_kernel_kconfig_directory_is_not_a_vendor_runtime` |
| driver-staging-no-module-load | 2 | `core:CommittedRecipe.test_recipe_declares_gpu_free_driver_staging`; `core:GuestInstall.test_driver_staging_does_not_try_to_load_a_gpu_module` |

**MEASURED, 2026-11-08:** [initial hygiene run](gates-initial.json) caught one unsupported attribution label in this new note. The label was corrected without modifying a gate or baseline. [Final hygiene exits](gates.json) cover documentation freshness, claim attribution, release claims, private references, GPU UUIDs, workflow lint and stock-patch policy, plus formatting/whitespace and the offline plan. [Source digests](source_hashes.json) identify the tested implementation.

**DESIGN:** coordinator qualification remains required: full build closure, independent complete-image scan, normal boot, offline provisioning, PINNED cross-verification across builds/boxes and GPU apps plus same-binary host reference. No real image, chroot, VM, GPU, rental, media upload or deployment ran here. Merging these fixes does not qualify an image for adoption/hosting. Old bash paths remain untouched.
