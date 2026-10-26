# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
function New-KfwcCaptureDirectory {
    param([Parameter(Mandatory)][string]$Path)
    # EXPECTED RED: collector requires a pre-existing, unused directory.
    return $Path
}
