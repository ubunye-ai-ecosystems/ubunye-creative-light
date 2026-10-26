<#
.SYNOPSIS
  Ubunye Light for Windows with ThabangVision collaboration (kopanod) inside: one MSI, one zip.

.DESCRIPTION
  1. Builds kopanod (the Kopano service, slice S4) from the Kopano lab sources in -KopanoLab
     (research/kopano/lab of ubunye-creative-studio-tools):
       - no console window (Windows GUI subsystem), so start at sign-in shows nothing;
       - the C runtime linked in (+crt-static), like Light: no Visual C++ redistributable;
       - a version resource that names it "ThabangVision collaboration" (what Task Manager and
         Startup apps show; people never see the protocol's name).
     Its version is kopanod's own (as ubunye/packaging/kopanod-version.sh: the lab's version and
     its number of commits), not Light's, so the newest kopanod wins whichever app brought it.
  2. Runs packaging/windows/package.ps1, which puts kopanod.exe in the MSI (collab.wxs: one
     shared copy in Program Files\ThabangVision\Collaboration for all Ubunye apps, with start at
     sign-in) and beside lightcraft.exe in the portable zip, and signs it with the other
     programs when signing secrets are set (sign.ps1, unchanged).

  Output: $env:DIST (default dist/release), as package.ps1 names it.
  Needs what package.ps1 needs (Rust MSVC, Windows SDK, WiX 5.0.2) and the lab sources.

.EXAMPLE
  pwsh ubunye/packaging/windows/collab-package.ps1 -Arch x64 -KopanoLab ..\ubunye-creative-studio-tools\research\kopano\lab
#>
param(
  [ValidateSet('x64', 'x86', 'arm64')] [string] $Arch = 'x64',
  [string] $KopanoLab = $env:KOPANO_LAB,
  [switch] $SkipBuild,
  [switch] $SkipKopanodBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $KopanoLab -or -not (Test-Path (Join-Path $KopanoLab 'kopano-node\Cargo.toml'))) {
  throw "-KopanoLab must be the research/kopano/lab folder (got '$KopanoLab')"
}
$KopanoLab = (Resolve-Path $KopanoLab).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

function Find-Rc {
  $onPath = Get-Command rc.exe -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
  $found = Get-ChildItem -Path $kits -Recurse -Filter rc.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\rc\.exe$' } | Sort-Object FullName -Descending | Select-Object -First 1
  if (-not $found) { throw 'rc.exe not found (install the Windows SDK)' }
  return $found.FullName
}

# kopanod's own version, as ubunye/packaging/kopanod-version.sh: the lab's workspace version and
# the number of commits that changed the lab (CI fetches its history). The MSI's file versioning
# then never replaces a newer kopanod.exe with an older one.
$LabVersion = $null
$inPkg = $false
foreach ($line in Get-Content (Join-Path $KopanoLab 'Cargo.toml')) {
  if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
  if ($inPkg -and $line -match '^\s*version\s*=\s*"([0-9.]+)') { $LabVersion = $Matches[1]; break }
}
if (-not $LabVersion) { throw "no [workspace.package] version in $KopanoLab\Cargo.toml" }
$Commits = "$(git -C $KopanoLab rev-list --count HEAD -- . 2>$null)".Trim()
if ($LASTEXITCODE -ne 0 -or $Commits -notmatch '^\d+$') { $Commits = '0' }
$Nums = @(($LabVersion -split '\.') + @('0', '0', '0') | Select-Object -First 3) + @($Commits)
$Version = $Nums -join '.'
Write-Output "kopanod $Version"
$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'x86' { 'i686-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$KTarget = if ($env:KOPANO_TARGET_DIR) { $env:KOPANO_TARGET_DIR } else { Join-Path $KopanoLab 'target' }
$Work = Join-Path $KTarget "light-collab\$Arch"
$Out = Join-Path $Work 'bin'
New-Item -ItemType Directory -Force -Path $Out | Out-Null

if (-not $SkipKopanodBuild) {
  # Version resource (numbers instead of winver.h names: no SDK include path needed).
  # FileDescription is the name Windows shows for the running service.
  $rcFile = Join-Path $Work 'kopanod.rc'
  $icon = (Join-Path $Root 'assets\app-icon\lightcraft.ico') -replace '\\', '\\'
  $v = $Nums -join ','
  $vs = $Nums -join '.'
  @"
1 ICON "$icon"
1 VERSIONINFO
FILEVERSION $v
PRODUCTVERSION $v
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Ubunye AI Ecosystems"
      VALUE "FileDescription", "ThabangVision collaboration"
      VALUE "FileVersion", "$vs"
      VALUE "InternalName", "kopanod"
      VALUE "OriginalFilename", "kopanod.exe"
      VALUE "ProductName", "ThabangVision"
      VALUE "ProductVersion", "$Version"
      VALUE "LegalCopyright", "MIT OR Apache-2.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"@ | Set-Content -Encoding ascii $rcFile
  $res = Join-Path $Work 'kopanod.res'
  $rc = Find-Rc
  Invoke-Native 'rc kopanod.rc' { & $rc /nologo /fo $res $rcFile }

  # Scoped to the target (as package.ps1 does), so build scripts and proc-macros are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  $before = [Environment]::GetEnvironmentVariable($flagVar)
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  $envTarget = $env:CARGO_TARGET_DIR
  $env:CARGO_TARGET_DIR = $KTarget
  Push-Location $KopanoLab
  try {
    # The link arguments only for kopanod itself (cargo rustc), not for the libraries it uses.
    Invoke-Native "cargo build kopanod ($Target)" {
      cargo rustc --release --locked -p kopano-node --bin kopanod --target $Target -- `
        -C link-arg=/SUBSYSTEM:WINDOWS -C link-arg=/ENTRY:mainCRTStartup -C "link-arg=$res"
    }
  } finally {
    Pop-Location
    $env:CARGO_TARGET_DIR = $envTarget
    [Environment]::SetEnvironmentVariable($flagVar, $before)
  }
  Copy-Item (Join-Path $KTarget "$Target\release\kopanod.exe") $Out -Force
}

# kopanod must be for this machine type and a GUI program (subsystem 2: no console window).
$kexe = Join-Path $Out 'kopanod.exe'
$bytes = [System.IO.File]::ReadAllBytes($kexe)
$pe = [BitConverter]::ToInt32($bytes, 0x3C)
$machine = [BitConverter]::ToUInt16($bytes, $pe + 4)
$subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C)
$want = switch ($Arch) { 'x64' { 0x8664 } 'x86' { 0x14C } 'arm64' { 0xAA64 } }
if ($machine -ne $want) { throw "kopanod.exe is for machine 0x$('{0:X}' -f $machine), expected $Arch" }
if ($subsystem -ne 2) { throw "kopanod.exe has PE subsystem $subsystem, expected 2 (no console window)" }
$desc = (Get-Item $kexe).VersionInfo.FileDescription
if ($desc -ne 'ThabangVision collaboration') { throw "kopanod.exe describes itself as '$desc'" }
Write-Output "ok kopanod.exe: $Arch, PE subsystem 2, '$desc'"

$env:UBUNYE_EXTRA_BIN_DIR = $Out
$env:UBUNYE_EXTRA_WXS = Join-Path $PSScriptRoot 'collab.wxs'
try {
  & (Join-Path $Root 'packaging\windows\package.ps1') -Arch $Arch -SkipBuild:$SkipBuild
} finally {
  Remove-Item Env:UBUNYE_EXTRA_BIN_DIR, Env:UBUNYE_EXTRA_WXS -ErrorAction SilentlyContinue
}
