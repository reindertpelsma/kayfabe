# LB1 Linux apps recipe evidence — 2026-11-07

STATUS: DESIGN-ONLY, 2026-11-07. MEASURED: GPU-free Python fixtures and public-input audits only; no real VM, GPU, image build, upload or deployment.

**MEASURED:** [input_heads.json](input_heads.json) records 88 anonymous HEAD observations: 84 matching lengths and four source archives without a length. [official_heads.json](official_heads.json) separately records all 29 official per-run URLs answering with matching lengths. These are availability observations, not SHA256 verification of those responses.

**MEASURED:** [download_hashes.json](download_hashes.json) records selected complete payload hashes. Other large recipe pins derive from upstream checksum/index metadata and were not all downloaded. [open_input_audits.json](open_input_audits.json) is a bounded filename/header audit of CPU torch and Blender, not a full installed-image scan or legal certification. [cupy_header_audit.json](cupy_header_audit.json) records the proprietary CUDA-header finding that moves CuPy to transient runtime provisioning; vendor bytes and header bodies are not included here.

**CODE READ:** the [design](../../docs/design/V3_APPS_IMAGE_RECIPE.md) records the final open-image/transient-NVIDIA policy, J4 dependency, shared-runtime build rules, CDP static-runtime exception, exact reproduction steps and limits. Old shell paths are untouched. Fake QEMU fixtures exercise actual argv launches, NoCloud seed decoding, bounded socket transcripts, marker/inventory checks and cleanup without a VM.

**DESIGN:** integrate J4; build twice; compare the second image against the first manifest; inspect complete scans/inventories; provision from official sources and run the existing apps gate and same-binary host reference. None of those real-image/hardware results is established here.

## GPU-free checks

**MEASURED:** 117 core image tests + 100 layer tests pass (217 new); the existing legacy payload tests pass 64/64 and matrix-table tests 12/12 (293 total in these suites). At the lane base the new image suites and mutation harness did not exist (CODE READ); a whole-repository before/after test total was not measured. No Rust implementation changed and no Rust build was run. Raw results: [core](tests-core.txt), [layers](tests-layers.txt), [legacy](tests-legacy-apps.txt), [matrix](tests-matrix.txt).

**MEASURED:** 52/52 mutants have at least two independently named **assertion failures**, with zero test errors. Syntax/import failures do not count as kills. The baseline passes first; every mutation runs in a scratch tree with its own fixture processes. [Complete machine-readable killers](mutants.json), [run output](mutants.txt). The table lists the count and two example named killers for each case.

| Mutated rule | Assertion killers | Two independent tests |
|---|---:|---|
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
| built-binary-pin | 2 | `core:Inventory.test_expected_binary_hash_enforced`; `core:Inventory.test_pinned_binary_cannot_change` |
| pip-freeze-check | 2 | `core:FakeQemu.test_build_refuses_pip_version_drift`; `core:Inventory.test_pip_drift_refused` |
| open-nvidia-boundary | 2 | `layers:LayerRecipe.test_nvidia_open_artifact_is_rejected`; `layers:LayerRecipe.test_nvidia_pip_wheel_cannot_be_moved_into_open_layer` |
| open-apt-components | 3 | `layers:LayerRecipe.test_multiverse_recipe_is_rejected`; `layers:LayerRecipe.test_restricted_or_multiverse_recipe_is_rejected` |
| open-scan-required | 5 | `layers:LayerInventories.test_clean_flag_cannot_hide_a_reported_proprietary_header`; `layers:LayerInventories.test_clean_flag_cannot_hide_a_reported_vendor_file` |
| official-receipts-required | 2 | `layers:LayerInventories.test_runtime_needs_actual_official_install_receipts`; `layers:LayerInventories.test_runtime_receipt_hash_cannot_drift` |
| inventory-comparison | 5 | `core:FakeQemu.test_verify_rejects_modified_app_binary`; `core:FakeQemu.test_verify_rejects_modified_pip_files` |
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
| unknown-notice-scan | 2 | `layers:StrictNoticeScan.test_nvidia_copyright_notice_is_not_silently_waived`; `layers:StrictNoticeScan.test_second_nvidia_notice_is_listed_as_a_finding` |
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

**MEASURED:** documentation freshness, release claims, private-reference, GPU-UUID and workflow-lint gates pass. The release ledger has 133 rows, zero errors and its three existing warnings. No baseline/debt file was changed. Pattern-based secret scanning of staged diffs checks private-key blocks, GitHub/AWS/Slack tokens and bearer literals; no matches were found. Formatting and diff-whitespace checks pass.

**CODE READ / source inspection:** all 36 selected CUDA-sample Makefiles were checked for explicit static device/runtime linkage; CDP quicksort is the one selected Makefile explicitly linking `cudadevrt`. The dynamic-link audit and complete licence scan still have to run on actual guest outputs. LB1 tests use fake executables on PATH and fake guest transcripts, not a real QEMU machine.
