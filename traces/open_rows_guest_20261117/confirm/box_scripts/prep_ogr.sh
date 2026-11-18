#!/bin/bash
# case lists (trusted --caselist: info cases first, then the case) and the run_apps.sh copy whose vkcts row takes the caselist pushed beside the tool
cd /workspace/ogr
sed 's#vkcts.py run --side \$SIDE --artifact \$B/vkcts --out \$O/vkcts#vkcts.py run --side $SIDE --artifact $B/vkcts --caselist $B/vkcts_tool/caselist.tsv --out $O/vkcts#' /workspace/kf-ci-src/scripts/apps/run_apps.sh > run_apps_ogr.sh
grep -c 'caselist \$B/vkcts_tool/caselist.tsv' run_apps_ogr.sh
INFO='info\tdEQP-VK.info.device_properties\ninfo\tdEQP-VK.info.device_extensions\n'
printf "$INFO"'dgc\tdEQP-VK.dgc.ext.graphics.multiview.view_mask_2.no_ies_gpl.indexed_draw_buffer_tokens\n' > list_dgc.tsv
printf "$INFO"'sparse\tdEQP-VK.sparse_resources.buffer.texel_buffers.sparse_residency.storage_texel_buffer_read_size_2_24_r32ui\n' > list_resid.tsv
printf "$INFO"'sparse\tdEQP-VK.sparse_resources.buffer.texel_buffers.sparse_binding.storage_texel_buffer_read_size_2_24_r32ui\n' > list_twin.tsv
cat list_twin.tsv
