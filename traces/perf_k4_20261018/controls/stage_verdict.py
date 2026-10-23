"""Strict report gate; an outer PASS never overrides a failed proxy stage."""
def whole_run_pass(result):
 outer=(result.get('verdict')=='PASS' and result.get('restored') is True
        and result.get('telemetry_exit')==0
        and result.get('host_stop',{}).get('new_host_xids')==0)
 proxy=result.get('proxy') or {}
 proxy_ok=proxy.get('verdict')=='PASS'
 phases_ok=proxy.get('phases')==12
 return outer and proxy_ok and phases_ok
