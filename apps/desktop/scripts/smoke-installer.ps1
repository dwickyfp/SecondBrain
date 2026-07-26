param(
  [Parameter(Mandatory=$true)][string]$Artifact,
  [Parameter(Mandatory=$true)][string]$Marker,
  [Parameter(Mandatory=$true)][string]$Memory,
  [Parameter(Mandatory=$true)][string]$Commit,
  [Parameter(Mandatory=$true)][string]$FixtureSha256
)
$ErrorActionPreference = 'Stop'
$extract = Join-Path $env:RUNNER_TEMP 'secondbrain-msi-smoke'
Remove-Item -Recurse -Force $extract -ErrorAction SilentlyContinue
Remove-Item -Force $Marker -ErrorAction SilentlyContinue
Remove-Item -Force $Memory -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $extract | Out-Null
$msi = Start-Process msiexec.exe -ArgumentList @('/a', (Resolve-Path $Artifact), '/qn', "TARGETDIR=$extract") -Wait -PassThru
if ($msi.ExitCode -ne 0) { throw "MSI administrative extraction failed: $($msi.ExitCode)" }
$executable = Get-ChildItem $extract -Recurse -Filter 'secondbrain-desktop.exe' | Select-Object -First 1
if (-not $executable) { throw 'secondbrain-desktop.exe was not present in the exact MSI' }
$env:SB_READINESS_MARKER = $Marker
$process = Start-Process $executable.FullName -PassThru
try {
  for ($attempt = 0; $attempt -lt 60 -and -not (Test-Path $Marker); $attempt++) {
    if ($process.HasExited) { throw "extracted application exited before readiness: $($process.ExitCode)" }
    Start-Sleep -Seconds 1
  }
  if (-not (Test-Path $Marker)) { throw 'readiness marker timed out' }
  node (Join-Path $PSScriptRoot 'verify-readiness.mjs') $Marker $Commit $FixtureSha256
  if ($LASTEXITCODE -ne 0) { throw 'readiness marker verification failed' }
  Start-Sleep -Seconds 5
  node (Join-Path $PSScriptRoot 'sample-process-tree-rss.mjs') $process.Id $Memory 500 20
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path $Memory)) { throw 'native process-tree RSS sampling failed' }
  if ($process.HasExited) { throw "extracted application exited during memory sampling: $($process.ExitCode)" }
} finally {
  if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
}
