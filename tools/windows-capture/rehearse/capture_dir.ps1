# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
function New-KfwcCaptureDirectory {
    param([Parameter(Mandatory)][string]$Path)
    if (Test-Path -LiteralPath $Path) { throw 'Capture directory already exists.' }
    New-Item -ItemType Directory -Path $Path -ErrorAction Stop | Out-Null
    return $Path
}
