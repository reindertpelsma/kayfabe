param([string[]]$Paths = @('C:\kf\enable580_once.ps1', 'C:\kf\bw8_enable_observe.ps1'))
$ErrorActionPreference = 'Stop'
$allErrors = @()
foreach ($path in $Paths) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Parser input missing: $path"
    }
    $tokens = $null
    $parseErrors = $null
    [System.Management.Automation.Language.Parser]::ParseFile(
        $path, [ref]$tokens, [ref]$parseErrors) | Out-Null
    foreach ($errorRecord in $parseErrors) {
        $allErrors += [ordered]@{
            Path = $path
            ErrorId = $errorRecord.ErrorId
            Message = $errorRecord.Message
            Extent = $errorRecord.Extent.Text
            StartLine = $errorRecord.Extent.StartLineNumber
            StartColumn = $errorRecord.Extent.StartColumnNumber
        }
    }
}
if ($allErrors.Count -gt 0) {
    $allErrors | ConvertTo-Json -Depth 6
    throw 'BW8 parser preflight refused'
}
'BW8_PARSER_GREEN=1'
