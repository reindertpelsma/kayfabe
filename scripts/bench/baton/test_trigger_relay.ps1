# Run in the guest against a separate host UDP fixture receiver, without GPU work.
param([string]$RelayPath = "$PSScriptRoot\hw2_trigger_relay.ps1", [int]$Port=39022)
$ErrorActionPreference='Stop'
$tokens=$null; $errors=$null
$null=[Management.Automation.Language.Parser]::ParseFile($RelayPath,[ref]$tokens,[ref]$errors)
if($errors.Count -ne 0) { throw ($errors | Out-String) }
& {
    # Override only the input boundary in this child scope; real trig.log is untouched.
    function Get-Content { param($Path,$Tail,[switch]$Wait)
        'x'*5000
        ([string][char]233)*2100
        'HW2_EMULATED 02:11:00.123 TRIGGER Heaven.exe silent 4s after 12 presents'
        'HW2_EMULATED 02:11:01.000 TRIGGER dwm.exe silent 5s after 14 presents'
    }
    & $RelayPath -Port $Port
}
@{version=$PSVersionTable.PSVersion.ToString();parser_errors=$errors.Count;expected_datagrams=3;fixture_only=$true} | ConvertTo-Json -Compress
