# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# hue_vibrance_guest.ps1 — guest side of scripts/bench/windows/hue_vibrance_lane.sh (PR #92, automated).
# Runs in the INTERACTIVE session (the lane starts it through C:\kf\ogl\runi.ps1, a one-shot scheduled
# task as the auto-logon user), because UI Automation and the test pattern need the user's desktop;
# results go to C:\kf\hv\<action>.json for the SYSTEM kfagent to read back.
#
# Actions:
#   pattern            show the full-screen test pattern (known patches + a frame counter) and keep it up
#   discover           dump the UI Automation tree of the NVIDIA colour UIs (names, types, ranges)
#   set <what> <value> set the NVIDIA app / NVIDIA Control Panel slider <what> (hue|vibrance) to <value>
#                      through UI Automation's RangeValuePattern (Microsoft's documented accessibility
#                      API on the vendor's own user control): no NvAPI, no registry, no driver internals
#   events <iso8601>   count nvlddmkm 153 and Display 4101 events since the given time
#
# Route facts are INFERRED until a run's discover dump confirms them: the NVIDIA app's slider names
# ("Hue", "Digital Vibrance"), its navigation ("System" -> "Displays"), the NVIDIA Control Panel page
# ("Adjust desktop color settings", sliders "Digital vibrance" and "Hue"). Each is a parameter
# ($env:HV_NAV, $env:HV_HUE_RE, $env:HV_VIB_RE) so a dump can correct them without editing code.
param([Parameter(Mandatory = $true)][string]$Action, [string]$A1 = '', [string]$A2 = '')
$ErrorActionPreference = 'Stop'
$Out = 'C:\kf\hv'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
function Emit($name, $obj) {
    $obj | ConvertTo-Json -Depth 6 -Compress | Set-Content -Encoding utf8 -Path (Join-Path $Out "$name.json")
}

# The pattern: the same table the host checker uses (hue_vibrance_check.py PATCHES; change both together).
# 1920x1080, background (128,128,128), 6 x 3 patches of 200x200 at x = 160 + 280*i, y = 120 + 280*j,
# counter box 600x120 at (660, 940): black digits on white, redrawn every 100 ms.
$Patches = @(
    @(255, 0, 0), @(0, 255, 0), @(0, 0, 255), @(255, 255, 0), @(0, 255, 255), @(255, 0, 255),
    @(192, 64, 64), @(64, 192, 64), @(64, 64, 192), @(224, 160, 128), @(96, 160, 224), @(160, 96, 32),
    @(0, 0, 0), @(64, 64, 64), @(128, 128, 128), @(192, 192, 192), @(255, 255, 255), @(32, 200, 96)
)

function Show-Pattern {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $f = New-Object System.Windows.Forms.Form
    $f.FormBorderStyle = 'None'; $f.TopMost = $true; $f.StartPosition = 'Manual'
    $f.Bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $f.BackColor = [System.Drawing.Color]::FromArgb(128, 128, 128)
    $f.ShowInTaskbar = $false; $f.Cursor = [System.Windows.Forms.Cursors]::Default
    $script:n = 0
    $f.Add_Paint({
        param($s, $e)
        $g = $e.Graphics
        for ($k = 0; $k -lt $Patches.Count; $k++) {
            $c = $Patches[$k]; $i = $k % 6; $j = [math]::Floor($k / 6)
            $b = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb($c[0], $c[1], $c[2]))
            $g.FillRectangle($b, 160 + 280 * $i, 120 + 280 * $j, 200, 200); $b.Dispose()
        }
        $g.FillRectangle([System.Drawing.Brushes]::White, 660, 940, 600, 120)
        $font = New-Object System.Drawing.Font('Consolas', 72, [System.Drawing.FontStyle]::Bold)
        $g.DrawString(('{0:D8}' -f $script:n), $font, [System.Drawing.Brushes]::Black, 670, 945); $font.Dispose()
    })
    $t = New-Object System.Windows.Forms.Timer
    $t.Interval = 100
    $t.Add_Tick({ $script:n++; $f.Invalidate((New-Object System.Drawing.Rectangle(660, 940, 600, 120))) })
    $t.Start()
    Emit 'pattern' @{ shown = $true; bounds = "$($f.Bounds)"; at = (Get-Date).ToString('o') }
    [System.Windows.Forms.Application]::Run($f)
}

function Load-Uia {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
}

# Top-level windows whose title matches.
function Get-Windows($re) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
    @($all | Where-Object { $_.Current.Name -match $re })
}

function Get-Descendants($el) {
    $el.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
}

# The candidate UIs: the NVIDIA app and the NVIDIA Control Panel (UWP or classic). Started if not running.
$AppRe = if ($env:HV_WINDOW_RE) { $env:HV_WINDOW_RE } else { '^(NVIDIA app|NVIDIA Control Panel)$' }
function Start-NvidiaUi {
    if ((Get-Windows $AppRe).Count -gt 0) { return 'running' }
    $cands = @(
        "$env:ProgramFiles\NVIDIA Corporation\NVIDIA app\CEF\NVIDIA app.exe",
        "$env:ProgramFiles\NVIDIA Corporation\NVIDIA App\CEF\NVIDIA App.exe",
        "$env:ProgramFiles\NVIDIA Corporation\Control Panel Client\nvcplui.exe"
    )
    foreach ($c in $cands) { if (Test-Path $c) { Start-Process -FilePath $c; Start-Sleep 8; return $c } }
    $pkg = Get-AppxPackage -Name 'NVIDIACorp.NVIDIAControlPanel' -ErrorAction SilentlyContinue
    if ($pkg) {
        Start-Process ("shell:AppsFolder\" + $pkg.PackageFamilyName + '!NVIDIACorp.NVIDIAControlPanel'); Start-Sleep 8
        return 'appx'
    }
    return 'none'
}

# Navigate: invoke / select each named item of $env:HV_NAV (';'-separated regexes) in order.
function Invoke-Nav($win) {
    $nav = if ($env:HV_NAV) { $env:HV_NAV } else { '^System$;^Displays?$;^Adjust desktop color settings$' }
    $done = @()
    foreach ($re in $nav.Split(';')) {
        $hit = @(Get-Descendants $win | Where-Object { $_.Current.Name -match $re }) | Select-Object -First 1
        if (-not $hit) { $done += "miss:$re"; continue }
        $p = $null
        foreach ($pat in @([System.Windows.Automation.InvokePattern]::Pattern,
                [System.Windows.Automation.SelectionItemPattern]::Pattern,
                [System.Windows.Automation.ExpandCollapsePattern]::Pattern)) {
            if ($hit.TryGetCurrentPattern($pat, [ref]$p)) { break }
        }
        if ($p -is [System.Windows.Automation.InvokePattern]) { $p.Invoke() }
        elseif ($p -is [System.Windows.Automation.SelectionItemPattern]) { $p.Select() }
        elseif ($p -is [System.Windows.Automation.ExpandCollapsePattern]) { $p.Expand() }
        $done += "hit:$re"
        Start-Sleep 2
    }
    $done
}

function Find-Slider($win, $re) {
    $ctl = [System.Windows.Automation.ControlType]::Slider
    @(Get-Descendants $win | Where-Object { $_.Current.ControlType -eq $ctl -and $_.Current.Name -match $re }) | Select-Object -First 1
}

function Do-Discover {
    Load-Uia
    $started = Start-NvidiaUi
    $wins = Get-Windows $AppRe
    $dump = @()
    foreach ($w in $wins) {
        $nav = Invoke-Nav $w
        foreach ($d in (Get-Descendants $w)) {
            $c = $d.Current
            $rv = $null; $range = $null
            if ($d.TryGetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern, [ref]$rv)) {
                $range = @{ min = $rv.Current.Minimum; max = $rv.Current.Maximum; value = $rv.Current.Value }
            }
            $dump += @{ window = $w.Current.Name; name = $c.Name; type = $c.ControlType.ProgrammaticName
                id = $c.AutomationId; cls = $c.ClassName; range = $range }
        }
    }
    Emit 'discover' @{ started = $started; windows = @($wins | ForEach-Object { $_.Current.Name }); nav = $nav; elements = $dump }
}

function Do-Set($what, [double]$value) {
    Load-Uia
    $re = switch ($what) {
        'hue' { if ($env:HV_HUE_RE) { $env:HV_HUE_RE } else { '^Hue' } }
        'vibrance' { if ($env:HV_VIB_RE) { $env:HV_VIB_RE } else { '(?i)^digital vibrance' } }
        default { throw "unknown control $what" }
    }
    $started = Start-NvidiaUi
    $res = @{ what = $what; requested = $value; started = $started; found = $false }
    foreach ($w in (Get-Windows $AppRe)) {
        $s = Find-Slider $w $re
        if (-not $s) { $res.nav = Invoke-Nav $w; $s = Find-Slider $w $re }
        if (-not $s) { continue }
        $rv = $s.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
        $res.found = $true; $res.window = $w.Current.Name; $res.name = $s.Current.Name
        $res.min = $rv.Current.Minimum; $res.max = $rv.Current.Maximum; $res.before = $rv.Current.Value
        $v = [math]::Max($res.min, [math]::Min($res.max, $value))
        $rv.SetValue($v)
        Start-Sleep -Milliseconds 800
        $res.after = $rv.Current.Value
        break
    }
    Emit "set-$what" $res
}

function Do-Events($since) {
    $t = [datetime]::Parse($since)
    $n153 = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'nvlddmkm'; Id = 153; StartTime = $t } -ErrorAction SilentlyContinue).Count
    $n4101 = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Display'; Id = 4101; StartTime = $t } -ErrorAction SilentlyContinue).Count
    Emit 'events' @{ since = $since; nvlddmkm_153 = $n153; display_4101 = $n4101; at = (Get-Date).ToString('o') }
}

try {
    switch ($Action) {
        'pattern' { Show-Pattern }
        'discover' { Do-Discover }
        'set' { Do-Set $A1 ([double]$A2) }
        'events' { Do-Events $A1 }
        default { throw "unknown action $Action" }
    }
} catch {
    Emit "error-$Action" @{ action = $Action; error = "$_"; at = (Get-Date).ToString('o') }
    exit 1
}
