# Writes latest.json, the signed update manifest. Upload it to the release next to Lighten.exe.
# Run it on the exact file you upload: code signing changes the bytes, so sign first.
#   .\scripts\make-latest.ps1 -Exe .\Lighten.exe -Version 0.1.2 -KeyFile .\keys\update.key
# Add -PreviousExe (the exact file of the last release) and -PreviousVersion to also write a small patch
# beside -Out. Upload the patch too; without it the app downloads the whole program.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
    [Parameter(Mandatory)][string]$KeyFile,
    [string]$Out = 'latest.json',
    [string]$PreviousExe,
    [ValidatePattern('^(\d+\.\d+\.\d+)?$')][string]$PreviousVersion
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
$base = "https://github.com/RDK456/leaveCalone/releases/download/v$Version"
$manifest = [ordered]@{ version = $Version; signature = $sig; full = [ordered]@{ url = "$base/Lighten.exe"; sha256 = $sha } }
$target = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
if ($PreviousExe -and $PreviousVersion) {
    $old = (Resolve-Path $PreviousExe).Path
    $name = "Lighten-$PreviousVersion-to-$Version.patch"
    $patch = Join-Path (Split-Path $target) $name
    $rebuilt = "$patch.check"
    $env:__COMPAT_LAYER = 'RunAsInvoker'
    try {
        Start-Process -FilePath $exePath -ArgumentList '--make-patch', "`"$old`"", "`"$exePath`"", "`"$patch`"" -Wait
        if (-not (Test-Path $patch)) { throw 'Making the patch failed; see make-patch-error.txt' }
        Start-Process -FilePath $exePath -ArgumentList '--apply-patch', "`"$old`"", "`"$rebuilt`"", "`"$patch`"" -Wait
        if (-not (Test-Path $rebuilt) -or (Get-FileHash $rebuilt -Algorithm SHA256).Hash.ToLower() -ne $sha) { throw 'The patch does not rebuild the new program' }
    } finally {
        $env:__COMPAT_LAYER = $before
        Remove-Item $rebuilt, "$patch.json" -Force -ErrorAction SilentlyContinue
    }
    $manifest.patches = [ordered]@{ $PreviousVersion = [ordered]@{ url = "$base/$name"; sha256 = (Get-FileHash $patch -Algorithm SHA256).Hash.ToLower() } }
    Write-Host "Wrote $patch ($((Get-Item $patch).Length) bytes)"
}
$json = $manifest | ConvertTo-Json -Depth 4
[IO.File]::WriteAllText($target, $json) # no byte-order mark: the app's JSON reader rejects one
Write-Host "Wrote $target (sha256 $sha)"
