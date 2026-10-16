import subprocess
from pathlib import Path
root=Path('/root/ada2build-payload')
for name in ['cup2.exe','cup3.exe','cup8.exe','kf_dxprobe.exe']:
 destination=str(__import__('pathlib').PureWindowsPath('C:/ProgramData/KayfabeSweep')/name)
 subprocess.run(['python3','/root/kayfabe/scripts/bench/diesweep/qga_file_put.py','/root/win/codex-ada2build/qga.sock',str(root/name),destination],check=True,timeout=135)
