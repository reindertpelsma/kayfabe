$f="C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool\b1\Saved\Config\Windows\GameUserSettings.ini"
Copy-Item $f "$f.kfbak" -Force
$t=Get-Content $f -Raw
$t=[regex]::Replace($t,'\("InsertFrame", "\d+"\)','("InsertFrame", "0")')
Set-Content $f $t -Encoding UTF8
Get-Content $f | Select-String 'UISettingData'
