#!/usr/bin/env python3
"""GPU-free: validate repository links/commands, shell syntax and declared install inputs."""
from pathlib import Path
import re
import subprocess
import sys

root=Path(__file__).resolve().parents[1]
paths=[root/'docs'/name for name in ['INSTALL.md','RUNNING_KAYFABE.md','SUPPORT_MATRIX.md']]
failures=[];blocks=links=commands=0
for path in paths:
    text=path.read_text()
    for link in re.findall(r'\]\(([^)]+)\)',text):
        if link.startswith(('https://','http://','#')):continue
        links+=1
        if not (path.parent/link.split('#')[0]).exists():failures.append(f'{path.name}: missing {link}')
    for block in re.findall(r'```sh\n(.*?)```',text,re.S):
        blocks+=1
        result=subprocess.run(['bash','-n'],input=block,text=True,capture_output=True)
        if result.returncode:failures.append(f'{path.name}: {result.stderr.strip()}')
        for command in re.findall(r'\b(scripts/[A-Za-z0-9_./-]+)',block):
            commands+=1
            if not (root/command).is_file():failures.append(f'{path.name}: missing command {command}')
    # Generic /path/to paths are explicitly documented operator inputs; prefixes are generated.
    if '/path/to/' in text and not any(word in text for word in ['operator-supplied','operator-supplied source']):
        failures.append(f'{path.name}: undeclared operator input')
print(f'{"FAIL" if failures else "PASS"}: {links} local links, {blocks} shell blocks, {commands} repository commands; no VM/install command executed')
for failure in failures:print(failure)
sys.exit(bool(failures))
