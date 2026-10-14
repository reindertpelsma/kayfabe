import datetime,json,pathlib,subprocess
argv=['python3','-u','/root/vast-windows-test/prepare/vfio_4070.py','run',
      '--inventory','/var/lib/vast-windows-test/inventory.json',
      '--ready','/var/lib/vast-windows-test/ready.json',
      '--workdir','/var/lib/vast-windows-4070-session']
pathlib.Path('/root/vw-vfio4070-command.json').write_text(json.dumps({'argv':argv,'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'source_revision':'537d96eb631861bb97e408daedd60c7eb27e8a27'},indent=2)+'\n')
result=subprocess.run(argv)
pathlib.Path('/root/vw-vfio4070-exit.json').write_text(json.dumps({'exitcode':result.returncode,'finished_utc':datetime.datetime.now(datetime.timezone.utc).isoformat()})+'\n')
raise SystemExit(result.returncode)
