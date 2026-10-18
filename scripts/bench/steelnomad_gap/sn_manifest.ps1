# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_manifest.ps1: dump the guest's raw DxgKrnl provider manifest (wevtutil gp, XML) to C:\kf\sn\manifest.xml and upload it
# to the host kfagent broker as sngap_manifestxml (decoded offline by analyze.py).
wevtutil gp Microsoft-Windows-DxgKrnl /ge:true /gm:true /f:xml | Set-Content -Encoding UTF8 C:\kf\sn\manifest.xml
& C:\kf\sn\sn_upload.ps1 -File C:\kf\sn\manifest.xml -Name sngap_manifestxml
