# Builds winget manifests for a published release, then you submit them to microsoft/winget-pkgs.
#   .\make-manifest.ps1 -Version 0.1.0
# Needs the release asset to exist: https://github.com/RDK456/leaveCalone/releases/download/v<Version>/Lighten.exe
param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
$id = 'RDK456.Lighten'
$url = "https://github.com/RDK456/leaveCalone/releases/download/v$Version/Lighten.exe"
$tmp = Join-Path $env:TEMP "Lighten-$Version.exe"
Invoke-WebRequest $url -OutFile $tmp
$sha = (Get-FileHash $tmp -Algorithm SHA256).Hash
$dir = Join-Path $PSScriptRoot "manifests\r\RDK456\Lighten\$Version"
New-Item -ItemType Directory -Force $dir | Out-Null
$schema = 'https://aka.ms/winget-manifest'
@"
# yaml-language-server: `$schema=$schema.version.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"@ | Set-Content "$dir\$id.yaml" -Encoding utf8
@"
# yaml-language-server: `$schema=$schema.installer.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
InstallerType: portable
Commands:
- lighten
ElevationRequirement: elevatesSelf
Installers:
- Architecture: x64
  InstallerUrl: $url
  InstallerSha256: $sha
ManifestType: installer
ManifestVersion: 1.6.0
"@ | Set-Content "$dir\$id.installer.yaml" -Encoding utf8
@"
# yaml-language-server: `$schema=$schema.defaultLocale.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
PackageLocale: en-US
Publisher: RDK456
PackageName: Lighten
PackageUrl: https://github.com/RDK456/leaveCalone
License: Proprietary
ShortDescription: Windows disk cleaner. Cleans junk, finds big and duplicate files, and moves folders to another drive without breaking anything.
Tags:
- disk-cleaner
- storage
- cleanup
- junction
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"@ | Set-Content "$dir\$id.locale.en-US.yaml" -Encoding utf8
winget validate --manifest $dir
Write-Host "Manifests in $dir. Test: winget install --manifest `"$dir`" (needs 'winget settings --enable LocalManifestFiles')."
