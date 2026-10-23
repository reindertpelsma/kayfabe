#!/bin/bash
set -euo pipefail
exec >/root/prov/observed_b1_build.log 2>&1
trap 'rc=$?; echo "OBSERVED_B1_BUILD_EXIT rc=$rc $(date -u -Is)"' EXIT
export PATH=/root/.cargo/bin:$PATH
python3 - <<'PY'
from pathlib import Path
import json,subprocess
root=Path('/root/kayfabe');work=Path('/workspace/bw-observed-b1')
meta=json.loads(Path('/root/sweep2/b1_merge.json').read_text())
parents=meta['parents']
subprocess.run(['git','-C',str(root),'fetch','--depth','128','origin',*parents],check=True)
subprocess.run(['git','-C',str(root),'worktree','add','--detach',str(work),parents[0]],check=True)
raw=Path('/root/sweep2/b1_commit.txt').read_bytes()
author=next(line for line in raw.decode().splitlines() if line.startswith('author '))
name,tail=author[7:].split(' <',1);email=tail.split('>',1)[0]
subprocess.run(['git','-C',str(work),'-c','user.name='+name,'-c','user.email='+email,
                'merge','--no-commit','--no-ff',parents[1]],check=True)
tree=subprocess.check_output(['git','-C',str(work),'write-tree'],text=True).strip()
assert tree==meta['tree'],'B1 merge tree differs from the prepared source'
source=subprocess.check_output(['git','-C',str(work),'hash-object','-t','commit','-w','--stdin'],
                               input=raw).decode().strip()
assert source==meta['source'],'B1 commit object differs'
subprocess.run(['git','-C',str(work),'reset','--hard',source],check=True)
assert not subprocess.check_output(['git','-C',str(work),'status','--porcelain'],text=True)
print('OBSERVED_B1_SOURCE',source,tree,flush=True)
PY
cd /workspace/bw-observed-b1
export CARGO_TARGET_DIR=/workspace/observed-b1-target CARGO_BUILD_JOBS=7
df -h /
KAYFABE_NO_KVM=1 nice -n 19 cargo test -j7 --release -p kf-chan -p kf-qemu -p kf-rm \
 >/root/prov/observed_b1_tests.log 2>&1
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 \
 /workspace/bench/qemu-build-observed-b1 >/root/prov/observed_b1_qemu_build.log 2>&1
python3 - <<'PY'
from pathlib import Path
import datetime,hashlib,json,re,subprocess
meta=json.loads(Path('/root/sweep2/b1_merge.json').read_text())
source=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
assert source==meta['source']
assert not subprocess.check_output(['git','status','--porcelain'],text=True)
binary=Path('/workspace/bench/kf3-bins')/source[:8]/'qemu-system-x86_64'
assert binary.is_file()
text=Path('/root/prov/observed_b1_tests.log').read_text()
counts=re.findall(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;',text)
assert counts and sum(int(n) for n,_ in counts)>0
assert 'test result: FAILED' not in text
row={'source':source,'tree':meta['tree'],'parents':meta['parents'],'binary':str(binary),
     'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'tests_passed':True,
     'test_passes':sum(int(n) for n,_ in counts),'test_ignored':sum(int(n) for _,n in counts),
     'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
     'scope':'exact measured page plus c130 Translated RC source; no further product cause',
     'test_log':'/root/prov/observed_b1_tests.log',
     'build_log':'/root/prov/observed_b1_qemu_build.log'}
Path('/root/prov/observed_b1_binary.json').write_text(json.dumps(row,indent=2)+'\n')
print(json.dumps(row))
PY
