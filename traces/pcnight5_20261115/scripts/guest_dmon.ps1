# guest-side nvidia-smi dmon, 1 s, runs until killed; started as a hidden background process
Start-Process -WindowStyle Hidden -FilePath "C:\Windows\System32\nvidia-smi.exe" -ArgumentList "dmon","-s","pucvmet","-d","1","-o","DT" -RedirectStandardOutput "C:\kfhw\guest_dmon.txt"
"started"
