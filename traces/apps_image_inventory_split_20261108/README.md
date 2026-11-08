STATUS: DESIGN-ONLY, 2026-11-08. CODE READ: inventory policy implementation and GPU-free fixtures only; no real image, chroot, VM or GPU qualification.

# LB1: PINNED identity and informational BUILT hashes

**DESIGN — owner correction, 2026-11-08:** compiled outputs may differ across boxes. Compare PINNED package versions, pip freeze with wheel hashes, downloaded input URLs/digests, source refs and recipe hash. Record compiled binaries/image hashes as BUILT information; require apps to be present. This supersedes the earlier per-binary equality rule and proposed strict level-2 acceptance gate. It does not waive download/transfer integrity, clean NVIDIA scans or guest completion checks.

**CODE READ:** recipe `2026-11-08.1`, manifest schema 3, has separate top-level `pinned` and `built` fields. `verify` and the shared `compare_inventory` report compare PINNED only and check required app presence. Verification prints both parts, `pinned_inventory_sha256`, actual image hash and reference image hash; no cross-build image/binary match flag gates the result. Runtime manifests follow the same split. Models/data copied from pinned inputs remain input hashes; pip installed-file digests are BUILT information. Exact hashes still protect a single build's audited export transfer. Schema-2 manifests require rebuilding with their appropriate checkout; no mixed legacy inventory is silently accepted.

**CODE READ:** [comparison-example.json](comparison-example.json) is explicitly synthetic: equal PINNED inputs with different compiled app and image hashes, accepted by the comparison helper. It is not an image measurement. The [design](../../docs/design/V3_APPS_IMAGE_RECIPE.md), [manual](../../docs/howto/linux-apps-image.md) and [chroot evaluation](../../docs/design/V3_APPS_IMAGE_CHROOT_EVALUATION.md) describe the corrected policy. QEMU remains the build method; no chroot backend or old bash path was removed.

**MEASURED, 2026-11-08 ([tests-first-red.txt](tests-first-red.txt)):** tests were changed before the production implementation. The initial targeted run had four assertion failures and 18 missing-new-API errors; that is red-baseline evidence, not mutant kills. Only named assertion failures count in the mutation results below; an error invalidates a mutant result.

## Final validation

**MEASURED, 2026-11-08 ([core](tests-core.txt), [layers](tests-layers.txt), [legacy](tests-legacy.txt), [matrix](tests-matrix.txt)):** 153 core + 127 layer + 64 legacy + 12 matrix tests pass: 356 selected tests. Before this correction the corresponding counts were 126 + 127 + 64 + 12 = 329, with evidence in the [earlier LB1 record](../apps_image_recipe_20261107/README.md). The 27-test net increase covers PINNED/BUILT separation, accepted binary/image differences, missing apps, input visibility/consistency and CLI hash output. No whole-repository test count or hardware result is claimed.

**MEASURED, 2026-11-08 ([mutant results](mutants.json), [run](mutants.txt)):** 70/70 mutants are killed by at least two named assertion failures with zero test errors. The previous 61 included a built-output hash rejection rule; the owner superseded that rule, and its replacement mutant reintroduces rejection and is killed by positive acceptance tests. Nine additional cases cover the new split. Both complete suites pass before selecting tests. An earlier ambiguous AST site and an earlier mutant run with fixture errors were refused; neither is reported as a valid kill. The current harness retains unique-site and zero-error requirements.

**MEASURED, 2026-11-08 ([gate exits](gates.json)):** documentation freshness, claim attribution, release claims, private references, GPU UUIDs, workflow lint and stock-patch policy all return zero. Release claims retain three existing warnings; no baseline was changed. Formatting and whitespace checks return zero. Production source digests are in [source_hashes.json](source_hashes.json).

| Mutated rule | Assertion killers | Two named tests |
|---|---:|---|
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
