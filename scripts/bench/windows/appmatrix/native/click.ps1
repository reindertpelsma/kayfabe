# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# click.ps1 -X px -Y px [-Key text]: move the cursor and left-click in the interactive session (run as a one-shot scheduled task), or type text.
param([int]$X = -1, [int]$Y = -1, [string]$Key = '')
Add-Type -AssemblyName System.Windows.Forms
Add-Type -Namespace K -Name M -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);'
if ($X -ge 0) { [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($X, $Y); Start-Sleep -Milliseconds 200; [K.M]::mouse_event(2, 0, 0, 0, 0); [K.M]::mouse_event(4, 0, 0, 0, 0) }
if ($Key) { [System.Windows.Forms.SendKeys]::SendWait($Key) }
