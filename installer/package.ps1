<#
  Builds pdfcraft_explorer_thumbs.dll against a PdfCraft checkout, and the MSI that installs it:

    powershell -ExecutionPolicy Bypass -File installer\package.ps1

  Output: ..\release\PdfCraft-Explorer-Thumbnails-<version>-x64.msi, and its SHA-256 beside it

  Nothing is written inside this repository or the PdfCraft checkout:

    <working folder>\
      pdfcraft\          the PdfCraft checkout             (-PdfCraft)
      explorer-thumbs\   this repository
      build\             cargo's build output              (-BuildDir)
      release\           the MSI                           (-OutDir)

  -PdfCraft <path>  the PdfCraft checkout to build against; default: ..\pdfcraft beside this repository
  -BuildDir <path>  where cargo builds; default: ..\build
  -OutDir <path>    where the MSI goes; default: ..\release
  -Test             also run this crate's format check, clippy and tests, and stop if they fail
  -SkipBuild        package the DLL already in the build folder
  -Pin              after a successful build, record the checkout's commit in pdfcraft.rev, the
                    commit GitHub builds releases against (.github/workflows/release.yml)

  Needs: Rust (MSVC toolchain) and WiX v5: dotnet tool install --global wix --version 5.0.2

  How it builds: this crate uses PdfCraft's renderer, so it has to be compiled as a member of
  PdfCraft's workspace (same dependency versions and patches). For the length of the build, the
  script links this repository into the checkout as crates\explorer-thumbs (a directory junction,
  which needs no administrator rights). PdfCraft's workspace takes every directory under crates\
  as a member. Afterwards the link is removed and Cargo.lock is put back, so the checkout is left
  exactly as it was.
#>
param(
  [string]$PdfCraft,
  [string]$BuildDir,
  [string]$OutDir,
  [switch]$Test,
  [switch]$SkipBuild,
  [switch]$Pin
)

$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Parent = Split-Path $Repo -Parent
if (-not $PdfCraft) { $PdfCraft = Join-Path $Parent 'pdfcraft' }
if (-not $BuildDir) { $BuildDir = Join-Path $Parent 'build' }
if (-not $OutDir) { $OutDir = Join-Path $Parent 'release' }
if (-not (Test-Path (Join-Path $PdfCraft 'crates\render\Cargo.toml'))) {
  throw "No PdfCraft checkout at '$PdfCraft' (crates\render is missing). Pass -PdfCraft <path>."
}
$PdfCraft = (Resolve-Path $PdfCraft).Path
# Absolute, because cargo runs against the checkout and wix from here.
$BuildDir = (New-Item -ItemType Directory -Force $BuildDir).FullName
$OutDir = (New-Item -ItemType Directory -Force $OutDir).FullName

function Invoke-Native([string]$What, [scriptblock]$Command) {
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "$What failed (exit code $LASTEXITCODE)" }
}

# This add-on's version, and the PdfCraft version and commit it is built against.
function Get-CargoVersion([string]$Manifest) {
  $Line = Select-String -Path $Manifest -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
  if (-not $Line) { throw "no version in $Manifest" }
  $Line.Matches[0].Groups[1].Value
}
$Version = Get-CargoVersion (Join-Path $Repo 'Cargo.toml')
$Against = Get-CargoVersion (Join-Path $PdfCraft 'Cargo.toml')
$Commit = (& git -C $PdfCraft rev-parse --short HEAD 2>$null)
if ($LASTEXITCODE -eq 0 -and $Commit) { $Against = "$Against ($Commit)" }
Write-Host "pdfcraft-explorer-thumbs $Version against PdfCraft $Against"

$Dll = Join-Path $BuildDir 'release\pdfcraft_explorer_thumbs.dll'

if (-not $SkipBuild) {
  $Link = Join-Path $PdfCraft 'crates\explorer-thumbs'
  if (Test-Path $Link) { throw "$Link already exists. Remove it (a leftover link is removed with: cmd /c rmdir `"$Link`")." }
  $Lock = Join-Path $PdfCraft 'Cargo.lock'
  $LockBefore = [IO.File]::ReadAllBytes($Lock)
  $Manifest = Join-Path $PdfCraft 'Cargo.toml'
  $OldTarget = $env:CARGO_TARGET_DIR
  try {
    Invoke-Native 'creating the link' { cmd /c mklink /J "$Link" "$Repo" | Out-Null }
    $env:CARGO_TARGET_DIR = $BuildDir
    if ($Test) {
      Invoke-Native 'cargo fmt --check' { cargo fmt --manifest-path $Manifest -p pdfcraft-explorer-thumbs --check }
      Invoke-Native 'cargo clippy' { cargo clippy --manifest-path $Manifest -p pdfcraft-explorer-thumbs --all-targets -- -D warnings }
      Invoke-Native 'cargo test' { cargo test --manifest-path $Manifest -p pdfcraft-explorer-thumbs }
    }
    Invoke-Native 'cargo build' { cargo build --release --manifest-path $Manifest -p pdfcraft-explorer-thumbs }
  }
  finally {
    $env:CARGO_TARGET_DIR = $OldTarget
    # rmdir without /s removes the link only, never what it points to.
    if (Test-Path $Link) { cmd /c rmdir "$Link" }
    [IO.File]::WriteAllBytes($Lock, $LockBefore)
  }
}
if (-not (Test-Path $Dll)) { throw "$Dll is missing: run without -SkipBuild" }

# The package is x64; refuse a DLL built for another machine type (0x8664 is x64).
$Bytes = [IO.File]::ReadAllBytes($Dll)
$Pe = [BitConverter]::ToInt32($Bytes, 0x3C)
if ([BitConverter]::ToUInt16($Bytes, $Pe + 4) -ne 0x8664) { throw "$Dll is not an x64 DLL" }

$Msi = Join-Path $OutDir "PdfCraft-Explorer-Thumbnails-$Version-x64.msi"

Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'thumbs.wxs') -arch x64 -d "Version=$Version" -d "Dll=$Dll" -d "Against=$Against" -o $Msi
}
# wix writes its debug symbols next to the MSI; they are of no use here.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))

# The MSI's SHA-256, in the "<hash>  <file name>" form checksum tools read.
$Hash = (Get-FileHash -Algorithm SHA256 $Msi).Hash.ToLower()
[IO.File]::WriteAllText("$Msi.sha256", "$Hash  $(Split-Path $Msi -Leaf)`n")

if ($Pin) {
  $Head = (& git -C $PdfCraft rev-parse HEAD)
  if ($LASTEXITCODE -ne 0 -or -not $Head) { throw "-Pin: '$PdfCraft' is not a git checkout" }
  [IO.File]::WriteAllText((Join-Path $Repo 'pdfcraft.rev'), "$Head`n")
  Write-Host "pdfcraft.rev: $Head"
}

Write-Output $Msi
