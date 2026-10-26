$ErrorActionPreference='Stop'
$cdb='C:\kf\windbg\amd64\cdb.exe'
if(!(Test-Path $cdb)){throw 'Official Microsoft cdb missing'}
$s=Get-AuthenticodeSignature $cdb
if($s.Status -ne 'Valid' -or $s.SignerCertificate.Subject -notlike '*Microsoft*'){throw "CDB signature refusal $($s.Status)"}
'PUBLIC_DEBUGGER '+((Get-FileHash $cdb -Algorithm SHA256).Hash)+' '+$s.SignerCertificate.Subject
$dest='C:\kf\dump_analysis';New-Item -ItemType Directory -Force -Path $dest|Out-Null
$dumps=@(Get-ChildItem C:\Windows\LiveKernelReports -Filter '*.dmp' -Recurse -ErrorAction SilentlyContinue | Sort-Object LastWriteTimeUtc | Select-Object -First 3)
$dumps+=@(Get-ChildItem C:\Windows\Minidump -Filter '*.dmp' -ErrorAction SilentlyContinue | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1)
if($dumps.Count -eq 0){throw 'No Windows crash dump to analyze'}
foreach($d in $dumps){
 $log=Join-Path $dest ($d.Name+'.txt'); $args=@('-y','srv*C:\kf\symbols*http://127.0.0.1:18765','-z',$d.FullName,'-logo',$log,'-c','.reload /f dxgkrnl.sys; !analyze -v; .bugcheck; q')
 & $cdb @args *> ($log+'.controller.txt');$rc=$LASTEXITCODE
 "DUMP_ANALYZED name=$($d.Name) bytes=$($d.Length) sha256=$((Get-FileHash $d.FullName -Algorithm SHA256).Hash) rc=$rc"
 if($rc -ne 0){throw "CDB failed $rc"}
 $txt=Get-Content $log -Raw
 $m=[regex]::Match($txt,'Arg1:\s*([0-9a-fA-F]{16})')
 if($m.Success){
  $ctx=$m.Groups[1].Value;$ctxlog=Join-Path $dest ($d.Name+'.tdr.txt')
  & $cdb -y 'srv*C:\kf\symbols*http://127.0.0.1:18765' -z $d.FullName -logo $ctxlog -c ".reload /f dxgkrnl.sys; dt dxgkrnl!_TDR_RECOVERY_CONTEXT $ctx; q" *> ($ctxlog+'.controller.txt')
  "PUBLIC_TDR_CONTEXT rc=$LASTEXITCODE file=$($d.Name)"
 }
}
'PUBLIC_DUMP_ANALYSIS_DONE'
