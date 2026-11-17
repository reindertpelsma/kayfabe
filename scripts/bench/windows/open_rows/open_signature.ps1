# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# open_signature.ps1 -Path FILE -- the Authenticode status and signer subject of a file as JSON (thin glue for kfopen_build.py; the judgement is Python).
param([Parameter(Mandatory = $true)][string]$Path)
Get-AuthenticodeSignature -LiteralPath $Path | Select-Object @{ n = 'Status'; e = { [string]$_.Status } }, @{ n = 'Subject'; e = { [string]$_.SignerCertificate.Subject } } | ConvertTo-Json -Compress
