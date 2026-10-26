<#
.SYNOPSIS
  Proof that Ubunye apps share one kopanod (ThabangVision collaboration) on Windows, on real MSIs.

.DESCRIPTION
  With two MSIs: install A, then B, uninstall A (kopanod.exe and its start-at-sign-in value stay,
  in Program Files\ThabangVision\Collaboration), uninstall B (both gone); then the same in the
  other order. With one MSI: install, check, uninstall. Quiet installs (msiexec /qn), so it needs
  an elevated shell: a CI runner, or a test PC. The service is started as it would be at sign-in
  and must survive removing the first app.

.EXAMPLE
  pwsh ubunye/packaging/windows/test-shared-install.ps1 light.msi photo.msi
#>
param([Parameter(Mandatory)] [string] $A, [string] $B)
$ErrorActionPreference = 'Stop'
$Shared = Join-Path $env:ProgramFiles 'ThabangVision\Collaboration\kopanod.exe'
$RunKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Run'
$RunValue = 'ThabangVision collaboration'
$Log = Join-Path $env:TEMP 'ubunye-collab-msi-logs'
New-Item -ItemType Directory -Force $Log | Out-Null

function Fail([string] $Why) { throw "FAIL: $Why" }
function Msi([string] $Op, [string] $Msi) {
  $name = [IO.Path]::GetFileNameWithoutExtension($Msi)
  $l = Join-Path $Log "$name-$Op.log"
  $p = Start-Process msiexec -ArgumentList "/$Op", "`"$Msi`"", '/qn', '/norestart', '/l*v', "`"$l`"" -Wait -PassThru
  if ($p.ExitCode -notin 0, 3010) { Get-Content $l -Tail 60; Fail "msiexec /$Op $name exited $($p.ExitCode)" }
  if ($p.ExitCode -eq 3010) { Fail "msiexec /$Op $name wants a restart (a file was in use)" }
  Write-Output "ok: msiexec /$Op $name"
}
function Answers { try { $c = [Net.Sockets.TcpClient]::new(); $c.Connect('127.0.0.1', 47100); $c.Close(); $true } catch { $false } }
function Start-AsAtSignIn {
  # what Windows does at sign-in: run the Run value's command
  $cmd = (Get-ItemProperty $RunKey).$RunValue
  $exe = ($cmd -split '"')[1]
  Start-Process $exe -ArgumentList 'run'
  foreach ($i in 1..30) { if (Answers) { Write-Output "ok: the service answers"; return }; Start-Sleep 1 }
  Fail "kopanod does not answer"
}
function Check-Shared {
  if (-not (Test-Path $Shared)) { Fail "$Shared missing" }
  $cmd = (Get-ItemProperty $RunKey -ErrorAction SilentlyContinue).$RunValue
  if ($cmd -ne "`"$Shared`" run") { Fail "Run value is '$cmd', expected '`"$Shared`" run'" }
  Write-Output "ok: $Shared ($((Get-Item $Shared).VersionInfo.FileVersion), '$((Get-Item $Shared).VersionInfo.FileDescription)') starts at sign-in"
}
function Check-Gone {
  if (Test-Path $Shared) { Fail "$Shared still there" }
  if ($null -ne (Get-ItemProperty $RunKey -ErrorAction SilentlyContinue).$RunValue) { Fail "Run value still there" }
  Start-Sleep 1
  if (Get-Process kopanod -ErrorAction SilentlyContinue) { Fail "kopanod still running" }
  if (-not (Test-Path (Join-Path $env:LOCALAPPDATA 'Kopano'))) { Fail "the data folder was removed" }
  Write-Output "ok: kopanod, its start at sign-in and the service are gone; the data folder stays"
}

$A = (Resolve-Path $A).Path
Write-Output "== install $A"
Msi i $A
Check-Shared
Start-AsAtSignIn
if (-not $B) {
  Write-Output "== uninstall $A"
  Msi x $A
  Check-Gone
  Write-Output 'PASS (one app)'
  exit 0
}
$B = (Resolve-Path $B).Path
foreach ($order in @(@($A, $B), @($B, $A))) {
  $first, $second = $order
  if ($first -ne $A) {
    Write-Output "== install $first"
    Msi i $first
    Check-Shared
    Start-AsAtSignIn
  }
  Write-Output "== install $second as well"
  Msi i $second
  Check-Shared
  # a newer kopanod.exe replaces the running one (stopped by the MSI); sign-in starts it again
  if (-not (Answers)) { Write-Output 'the newer kopanod replaced the running one'; Start-AsAtSignIn }
  Write-Output "== uninstall $first: $second keeps collaboration"
  Msi x $first
  Check-Shared
  if (-not (Answers)) { Fail "uninstalling the first app stopped the service" }
  Write-Output 'ok: the service still answers'
  Write-Output "== uninstall $second: the last app"
  Msi x $second
  Check-Gone
}
Write-Output 'PASS (two apps, both orders)'
