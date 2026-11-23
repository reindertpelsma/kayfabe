| attempt | platform | duration s | Resolve alive at last soak poll s | dead polls | Application Error 1000 after clock base | Resolve/nvoglv64 events | LKE/nvlddmkm | host Xid | kf3 unserviced | kf3 refused |
|---|---|---|---|---|---|---|---|---|---|---|
| s1v_a1 | vfio | 1139 | 936 | 0 | 0 | 0 | 0 | 0 | n/a | n/a |
| s1v_a2 | vfio | 1148 | 940 | 0 | 0 | 0 | 0 | 0 | n/a | n/a |
| s1v_a3 | vfio | 1122 | 915 | 0 | 0 | 0 | 0 | 0 | n/a | n/a |
| s2k_a1 | kf3 | 1318 | 1131 | 0 | 0 | 0 | 0 | 0 | 3036 | 856 |
| s2k_a2 | kf3 | 1334 | 1142 | 0 | 0 | 0 | 0 | 0 | 3045 | 873 |
| s2k_a3 | kf3 | 1328 | 1142 | 0 | 0 | 0 | 0 | 0 | 3041 | 786 |

s1v_a1 app:resolve FAIL FILE h264: format step failed; add to queue step failed; render step fail
s1v_a2 app:resolve FAIL FILE h264: format step failed; add to queue step failed; render step fail
s1v_a3 app:resolve FAIL FILE h264: format step failed; add to queue step failed; render step fail
s2k_a1 app:resolve FAIL FILE dnxhr: render step failed; render start step failed; guest: only 0 s
s2k_a2 app:resolve FAIL FILE dnxhr: render step failed; render start step failed; guest: only 0 s
s2k_a3 app:resolve FAIL FILE dnxhr: render step failed; render start step failed; guest: only 0 s
