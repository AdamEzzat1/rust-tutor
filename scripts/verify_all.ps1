# Definition-of-done check (Windows PowerShell). Same steps as verify_all.sh.
# Usage: scripts\verify_all.ps1 [-Fast]   (-Fast skips holemap, the slow step)
param([switch]$Fast)
Set-Location (Join-Path $PSScriptRoot "..")
$failed = @()
$T = "target\release\tutor.exe"

function Invoke-Step([string]$Name, [scriptblock]$Body) {
    Write-Output "== $Name"
    & $Body
    if ($LASTEXITCODE -eq 0) { Write-Output "   PASS" }
    else { Write-Output "   FAIL"; $script:failed += $Name }
}

Invoke-Step "toolchain pinned" {
    $v = (rustc --version).Split(" ")[1]
    $pin = (Get-Content RUST_VERSION -Raw).Trim()
    if ($v -eq $pin) { $global:LASTEXITCODE = 0 } else { $global:LASTEXITCODE = 1 }
}
Invoke-Step "build"           { cargo build --release --quiet }
Invoke-Step "tutor tests"     { cargo test --quiet }
Invoke-Step "clippy (tutor)"  { cargo clippy --quiet --all-targets -- -D warnings }
Invoke-Step "odeint tests"    { Push-Location projects\odeint\reference; cargo test --quiet; $c = $LASTEXITCODE; Pop-Location; $global:LASTEXITCODE = $c }
Invoke-Step "clippy (odeint)" { Push-Location projects\odeint\reference; cargo clippy --quiet --all-targets -- -D warnings; $c = $LASTEXITCODE; Pop-Location; $global:LASTEXITCODE = $c }
Invoke-Step "reference clean" { & $T check projects\odeint\reference\src projects\odeint\reference\tests }
Invoke-Step "scaffold"        { & $T scaffold odeint --check }
Invoke-Step "baseline"        { & $T baseline odeint --check }
if (-not $Fast) {
    Invoke-Step "holemap"     { & $T holemap odeint --check }
}
Invoke-Step "fixtures"        { & $T fixtures odeint --check }
Invoke-Step "lessons"         { & $T lessons verify }
Invoke-Step "course"          { & $T course verify }

Write-Output ""
if ($failed.Count -eq 0) { Write-Output "verify_all: PASS"; exit 0 }
Write-Output ("verify_all: FAIL (" + ($failed -join ", ") + ")")
exit 1
