from pathlib import Path
import json,sys
sys.path.insert(0,'/root/sweep2/guard/blackwell')
import phase_xid_guard as G
pid=int(Path('/workspace/bw-capc-validation/kfwin/run/qemu.pid').read_text());p=Path('/proc')/str(pid)
phase={'arm':sys.argv[1],'pid':pid,'exe':sys.argv[2],'starttime':int((p/'stat').read_text().rpartition(')')[2].split()[19]),'pci':Path('/root/prov/pci.txt').read_text().strip(),'rpc_path':None}
phase['expected_pci']=phase['pci'];G.validate_phase_owner(phase);G.atomic_json('/root/prov/phase.json',phase)
