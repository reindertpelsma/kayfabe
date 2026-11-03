# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# inf_cover.ps1 — which candidate dies does the image's NVIDIA driver package (DriverStore nv_disp*.inf) list?
$ids = @{ '2484'='RTX 3070 GA104'; '24B0'='RTX A4000 GA104GL'; '2206'='RTX 3080 GA102'; '2503'='RTX 3060 GA106';
          '2786'='RTX 4070 AD104'; '2803'='RTX 4060 Ti AD106'; '2805'='RTX 4060 Ti 16G AD106'; '2705'='RTX 4070 Ti S AD103'; '2684'='RTX 4090 AD102';
          '2D04'='RTX 5060 Ti GB206'; '2C05'='RTX 5070 Ti GB203'; '2B85'='RTX 5090 GB202'; '1E87'='RTX 2080 TU104'; '1F08'='RTX 2060 TU106'; '1EB8'='T4 TU104' }
foreach ($inf in @(Get-ChildItem C:\Windows\System32\DriverStore\FileRepository -Recurse -Filter 'nv_disp*.inf' -ErrorAction SilentlyContinue)) {
    $txt = Get-Content $inf.FullName -Raw
    foreach ($k in ($ids.Keys | Sort-Object)) {
        Write-Output ("INFCOVER {0} DEV_{1} {2} listed={3}" -f $inf.Name, $k, $ids[$k], ($txt -match ("PCI\\VEN_10DE&DEV_" + $k)))
    }
}
