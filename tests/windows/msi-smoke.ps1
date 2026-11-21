# SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
# SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception
#
# Silent MSI install smoke test (R43), run on a Windows CI runner by
# .github/workflows/installers.yml after it builds the MSI:
#
#   tests/windows/msi-smoke.ps1 -Msi <path> -Cli <panic-button.exe>
#
# An invalid token fails the install and leaves nothing behind. A silent
# install with every property writes them to the machine config, copies the
# token (and keeps it out of the MSI log), adds the firewall rules and the
# Run key. The app enrols from the token and blanks it, registers its
# watchdog task and ignores a watchdog start while running. Uninstalling
# with PURGE=1 removes the rules, the Run key, the task and
# %ProgramData%\Panic Button.

param(
  [Parameter(Mandatory = $true)][string]$Msi,
  [Parameter(Mandatory = $true)][string]$Cli
)

$ErrorActionPreference = "Stop"
$failures = @()
function Check([string]$name, [bool]$ok, [string]$detail = "") {
  if ($ok) { Write-Host "ok   $name" } else { Write-Host "FAIL $name $detail"; $script:failures += $name }
}

$work = Join-Path $env:RUNNER_TEMP "msi-smoke"
New-Item -ItemType Directory -Force -Path $work | Out-Null
$data = Join-Path $env:ProgramData "Panic Button"
$app = Join-Path $env:ProgramFiles "Panic Button\panic-button-desktop.exe"
$run = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Run"

function Install([string[]]$extra, [string]$log) {
  $args = @("/i", "`"$Msi`"", "/qn", "/l*v", "`"$log`"") + $extra
  (Start-Process msiexec.exe -ArgumentList $args -Wait -PassThru).ExitCode
}

# A practice and an enrolment code, made with the CLI.
$env:PANIC_BUTTON_PASSPHRASE = "a long ci passphrase"
& $Cli --data-dir "$work\admin" practice init --name "CI Surgery" --recovery-kit "$work\kit" | Out-Null
& $Cli --data-dir "$work\admin" token create --days 1 --label "msi smoke" --out "$work\practice.token" | Out-Null
$token = (Get-Content "$work\practice.token" -Raw).Trim()
Check "test practice and token" ($token.StartsWith("panic-button-enrol-1:"))

# An invalid token fails the install and leaves nothing installed.
Set-Content -Path "$work\bad.token" -Value "not a token"
$code = Install @("ENROLMENT_TOKEN_FILE=`"$work\bad.token`"") "$work\bad.log"
Check "invalid token fails the install" ($code -eq 1603) "exit $code"
Check "nothing installed after the failure" (-not (Test-Path $app))
Check "no config after the failure" (-not (Test-Path "$data\config.toml"))

# A silent install with every property.
$code = Install @(
  "ENROLMENT_TOKEN_FILE=`"$work\practice.token`"",
  "ROOM=`"Consulting Room 4`"",
  "RELAY_URL=wss://relay.example/v1/ws",
  "SEED_PEERS=`"10.0.0.5:47474,10.0.0.6:47474`""
) "$work\install.log"
Check "install succeeds" ($code -eq 0) "exit $code"
Check "program installed" (Test-Path $app)
$config = if (Test-Path "$data\config.toml") { Get-Content "$data\config.toml" -Raw } else { "" }
Write-Host $config
Check "config has the room" ($config -match 'room = "Consulting Room 4"')
Check "config has the relay" ($config -match 'wss://relay.example/v1/ws')
Check "config has both seed peers" ($config -match '10.0.0.5:47474' -and $config -match '10.0.0.6:47474')
Check "config names the copied token" ($config -match 'enrol\.token')
Check "config does not hold the token" (-not $config.Contains($token))
$copied = if (Test-Path "$data\enrol.token") { (Get-Content "$data\enrol.token" -Raw).Trim() } else { "" }
Check "token copied to enrol.token" ($copied -eq $token)
$log = Get-Content "$work\install.log" -Raw
Check "token kept out of the MSI log" (-not $log.Contains($token))
$rules = @(Get-NetFirewallRule -DisplayName "Panic Button" -ErrorAction SilentlyContinue)
Check "two firewall rules" ($rules.Count -eq 2) "found $($rules.Count)"
$profiles = ($rules | ForEach-Object { $_.Profile.ToString() }) -join ","
Check "rules on Domain and Private only" ($profiles -notmatch "Public" -and $profiles -notmatch "Any") $profiles
$runValue = (Get-ItemProperty -Path $run -Name "Panic Button" -ErrorAction SilentlyContinue)."Panic Button"
Check "Run key starts the app at sign-in" ($runValue -like "*panic-button-desktop.exe*") $runValue

# The app: enrols from the token, registers the watchdog, ignores a watchdog start.
$p = Start-Process $app -PassThru
$enrolled = $false
for ($i = 0; $i -lt 60 -and -not $enrolled; $i++) {
  Start-Sleep -Seconds 2
  $out = & $Cli --format json ctl join-status 2>$null
  if ($LASTEXITCODE -eq 0 -and ($out | ConvertFrom-Json).enrolled) { $enrolled = $true }
}
Check "app enrolled from the MSI token" $enrolled
$left = if (Test-Path "$data\enrol.token") { (Get-Content "$data\enrol.token" -Raw) } else { "" }
Check "token blanked after enrolment" ([string]::IsNullOrWhiteSpace($left)) "$($left.Length) characters left"
$task = @(Get-ScheduledTask -TaskName "Panic Button watchdog*" -ErrorAction SilentlyContinue)
Check "watchdog task registered" ($task.Count -ge 1)
if ($task.Count -ge 1) {
  $action = $task[0].Actions[0]
  Check "watchdog runs the app with --ensure-running" ($action.Execute -like "*panic-button-desktop.exe*" -and $action.Arguments -match "ensure-running") "$($action.Execute) $($action.Arguments)"
}
$second = Start-Process $app -ArgumentList "--ensure-running" -PassThru
$exited = $second.WaitForExit(20000)
Check "a watchdog start exits while the app runs" $exited
Check "the app is still running" (-not $p.HasExited)
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Get-Process panic-button-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

# Uninstall with PURGE=1.
$code = (Start-Process msiexec.exe -ArgumentList @("/x", "`"$Msi`"", "/qn", "PURGE=1", "/l*v", "`"$work\uninstall.log`"") -Wait -PassThru).ExitCode
Check "uninstall succeeds" ($code -eq 0) "exit $code"
Check "program removed" (-not (Test-Path $app))
Check "firewall rules removed" (@(Get-NetFirewallRule -DisplayName "Panic Button" -ErrorAction SilentlyContinue).Count -eq 0)
Check "Run key removed" ($null -eq (Get-ItemProperty -Path $run -Name "Panic Button" -ErrorAction SilentlyContinue))
Check "watchdog task removed" (@(Get-ScheduledTask -TaskName "Panic Button watchdog*" -ErrorAction SilentlyContinue).Count -eq 0)
Check "PURGE=1 removed ProgramData\Panic Button" (-not (Test-Path $data))

if ($failures.Count -gt 0) {
  Write-Host "FAILURES: $($failures -join ', ')"
  exit 1
}
Write-Host "all passed"
