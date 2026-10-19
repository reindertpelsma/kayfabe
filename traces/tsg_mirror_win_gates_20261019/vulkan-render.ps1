$ErrorActionPreference='Stop'
"VULKAN_RENDER_BEGIN $((Get-Date).ToString('o'))" | Set-Content C:\kf-m2\vulkan-render.log
$a=@('-raster','1','-temporal','1','-fps','1','-info','1','-benchmark','1','-close','1','-count','1','-asteroids','20000','-width','1280','-height','720','-fullscreen','0','-vk','-image','C:\kf-m2\vulkan.png','-times','C:\kf-m2\vulkan.times.txt')
"COMMAND GravityMark.exe $($a -join ' ')" | Add-Content C:\kf-m2\vulkan-render.log
$p=Start-Process -FilePath C:\kf-m2\GravityMark\bin\GravityMark.exe -ArgumentList $a -WorkingDirectory C:\kf-m2\GravityMark\bin -PassThru -RedirectStandardOutput C:\kf-m2\vulkan.stdout.txt -RedirectStandardError C:\kf-m2\vulkan.stderr.txt
$p.Id | Set-Content C:\kf-m2\vulkan.pid
$p.WaitForExit()
"VULKAN_RENDER_END rc=$($p.ExitCode) $((Get-Date).ToString('o'))" | Add-Content C:\kf-m2\vulkan-render.log
$p.ExitCode | Set-Content C:\kf-m2\vulkan.rc
