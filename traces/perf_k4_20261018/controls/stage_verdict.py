"""Original outer runner predicate, preserved to reproduce its false PASS."""
def whole_run_pass(result):
 return (result.get('verdict')=='PASS' and result.get('restored') is True
         and result.get('telemetry_exit')==0
         and result.get('host_stop',{}).get('new_host_xids')==0)
