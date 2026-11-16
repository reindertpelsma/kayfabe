# Acrobat: open the data volume's PDF through its automation interface, count pages, extract five pages into a new PDF, quit.
. 'C:\kfhw\lib.ps1'
$R = Init-Run 'acrobat'
try {
  $DATA = Data-Root
  $pdf = Find-Asset 'apollo11_anomaly_report_3.pdf'
  $R.source = $pdf.Name
  Step 'com'
  $app = $null
  for ($i = 0; $i -lt 10 -and -not $app; $i++) { try { $app = New-Object -ComObject AcroExch.App } catch { Start-Sleep 5; [void](Dismiss-Dialogs 'Acrobat') } }
  if (-not $app) { throw 'Acrobat COM not available' }
  [void](Wait-For { $false } 'Acrobat' 10)
  $av = New-Object -ComObject AcroExch.AVDoc
  $opened = $av.Open($pdf.FullName, $pdf.Name)
  $R.opened = [bool]$opened
  $pd = $av.GetPDDoc()
  $R.pages = $pd.GetNumPages()
  $out = New-Object -ComObject AcroExch.PDDoc
  [void]$out.Create()
  $ok = $out.InsertPages(-1, $pd, 0, 5, 0)
  $R.extract_ok = [bool]$ok
  $R.extracted_pages = $out.GetNumPages()
  $saved = $out.Save(1, "$D\out.pdf")
  $R.saved = [bool]$saved
  $out.Close() | Out-Null
  $av.Close(1) | Out-Null
  $R.outputs = @(File-Info "$D\out.pdf")
  try { [void]$app.Exit() } catch {}
  $R.quit = Quit-App 'Acrobat'
} catch { $R.error = $_.Exception.Message; $R.quit = Quit-App 'Acrobat' }
Finish $R
