APPRES side=guest app=UnifiedMemoryStreams verdict=HANG rc=ssh127 secs=- quiet=- note=no-result-line(guest_alive=1) gpu_use=NA gpu_auth=none gpu_subs=0 gpu_births=2 boot=ap_c2um_off_b1 guest_xid=0 kf3_lines=5 kf3_refusals=0
APPS_WEDGE boot=ap_c2um_off_b1 after=UnifiedMemoryStreams sanity_vectorAdd=0
APPRES side=guest app=UnifiedMemoryPerf verdict=NOTRUN rc=127 secs=0 quiet=0 note=bash: line 1: /opt/apps/bundle/samples/UnifiedMemoryPerf: No such file or directory gpu_use=NA gpu_auth=none gpu_subs=0 gpu_births=2 boot=ap_c2um_off_b2 guest_xid=0 kf3_lines=4 kf3_refusals=0
APPS_WEDGE boot=ap_c2um_off_b2 after=UnifiedMemoryPerf sanity_vectorAdd=0
APPRES side=guest app=conjugateGradientUM verdict=NOTRUN rc=127 secs=0 quiet=0 note=bash: line 1: /opt/apps/bundle/samples/conjugateGradientUM: No such file or directory gpu_use=NA gpu_auth=none gpu_subs=0 gpu_births=2 boot=ap_c2um_off_b3 guest_xid=0 kf3_lines=5 kf3_refusals=0
APPS_WEDGE boot=ap_c2um_off_b3 after=conjugateGradientUM sanity_vectorAdd=0
