import json,pathlib,subprocess,time
work=pathlib.Path('/var/lib/vast-windows-test')
argv=['python3','-u','/root/vast-windows-test/prepare/prepare.py','--inventory',str(work/'inventory.json'),'--workdir',str(work),'--cache','/var/lib/vast-windows-test-media','--resume','--login-test-hold','3600','--memory-mib','8192','--cpus','8']
(work/'job-command.json').write_text(json.dumps({'argv':argv,'started':time.time(),'source_revision':subprocess.check_output(['git','-C','/root/vast-windows-test','rev-parse','HEAD'],text=True).strip()},indent=2)+'\n')
result=subprocess.run(argv)
(work/'prepare-exit.json').write_text(json.dumps({'exitcode':result.returncode,'finished':time.time()})+'\n')
raise SystemExit(result.returncode)
