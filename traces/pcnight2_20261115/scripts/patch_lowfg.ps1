$f="C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool\b1\Saved\Config\Windows\GameUserSettings.ini"
Copy-Item $f "$f.kfbak" -Force
$t=Get-Content $f -Raw
foreach($k in "__none__"){ $t=[regex]::Replace($t,'\("'+$k+'", "\d+"\)','("'+$k+'", "0")') }
foreach($k in "QualityLevel","ViewDistance","AntiAliasing","PostProcessing","ShadowQuality","TextureQuality","FxQuality","MaterialQuality","VegetationQuality","GlobalIllumination","ReflectionQuality"){ $t=[regex]::Replace($t,'\("'+$k+'", "\d+"\)','("'+$k+'", "0")') }
$t=[regex]::Replace($t,'(?m)^(sg\.(?!ResolutionQuality|RayTracing)\w+)=\d+','${1}=0')
Set-Content $f $t -Encoding UTF8
Get-Content $f | Select-String 'UISettingData','sg\.'
