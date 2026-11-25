# Writes latest.json, the signed update manifest. Upload it to the release next to Lighten.exe.
# Run it on the exact file you upload: code signing changes the bytes, so sign first.
#   .\scripts\make-latest.ps1 -Exe .\Lighten.exe -Version 0.1.2 -KeyFile .\keys\update.key
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
    [Parameter(Mandatory)][string]$KeyFile,
    [string]$Out = 'latest.json'
)
$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path
$keyPath = (Resolve-Path $KeyFile).Path
$sha = (Get-FileHash $exePath -Algorithm SHA256).Hash.ToLower()
$sigFile = Join-Path ([IO.Path]::GetTempPath()) "lighten-sig-$PID.txt"
$before = $env:__COMPAT_LAYER
$env:__COMPAT_LAYER = 'RunAsInvoker' # signing needs no administrator rights
try {
    Start-Process -FilePath $exePath -ArgumentList '--sign', "`"$keyPath`"", $Version, $sha, "`"$sigFile`"" -Wait
    if (-not (Test-Path $sigFile)) { throw 'Signing failed; see sign-error.txt' }
    $sig = (Get-Content $sigFile -Raw).Trim()
} finally {
    $env:__COMPAT_LAYER = $before
    Remove-Item $sigFile -Force -ErrorAction SilentlyContinue
}
$url = "https://github.com/RDK456/leaveCalone/releases/download/v$Version/Lighten.exe"
$json = [ordered]@{ version = $Version; signature = $sig; full = [ordered]@{ url = $url; sha256 = $sha } } | ConvertTo-Json
$target = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
[IO.File]::WriteAllText($target, $json) # no byte-order mark: the app's JSON reader rejects one
Write-Host "Wrote $target (sha256 $sha)"
