Get-ChildItem C:\Users -Recurse -Filter *.3dmark-result -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | ForEach-Object { "RESULT $($_.Name) $($_.LastWriteTimeUtc.ToString('o'))" }
"RESULTS_END"
