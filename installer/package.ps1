<#
  Builds pdfcraft_explorer_thumbs.dll and the MSI that installs it:

    powershell -ExecutionPolicy Bypass -File crates\explorer-thumbs\installer\package.ps1

  Output: target\explorer-thumbs\PdfCraft-Explorer-Thumbnails-<version>-x64.msi

  Needs: Rust (MSVC toolchain) and WiX v5: dotnet tool install --global wix --version 5.0.2
  -SkipBuild packages the DLL already in target\release.
#>
param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$Crate = Join-Path $Root 'crates\explorer-thumbs'

function Invoke-Native([string]$What, [scriptblock]$Command) {
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "$What failed (exit code $LASTEXITCODE)" }
}

# The add-on's own version, from its Cargo.toml.
$VersionLine = Select-String -Path (Join-Path $Crate 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $VersionLine) { throw 'no version in crates\explorer-thumbs\Cargo.toml' }
$Version = $VersionLine.Matches[0].Groups[1].Value

if (-not $SkipBuild) {
  Invoke-Native 'cargo build' { cargo build --release --manifest-path (Join-Path $Root 'Cargo.toml') -p pdfcraft-explorer-thumbs }
}
$Dll = Join-Path $Root 'target\release\pdfcraft_explorer_thumbs.dll'
if (-not (Test-Path $Dll)) { throw "$Dll is missing: run without -SkipBuild" }

# The package is x64; refuse a DLL built for another machine type (0x8664 is x64).
$Bytes = [IO.File]::ReadAllBytes($Dll)
$Pe = [BitConverter]::ToInt32($Bytes, 0x3C)
if ([BitConverter]::ToUInt16($Bytes, $Pe + 4) -ne 0x8664) { throw "$Dll is not an x64 DLL" }

$OutDir = Join-Path $Root 'target\explorer-thumbs'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$Msi = Join-Path $OutDir "PdfCraft-Explorer-Thumbnails-$Version-x64.msi"

Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'thumbs.wxs') -arch x64 -d "Version=$Version" -d "Dll=$Dll" -o $Msi
}
# wix writes its debug symbols next to the MSI; they are of no use here.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))

Write-Output $Msi
