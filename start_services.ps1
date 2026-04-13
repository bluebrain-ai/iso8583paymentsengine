param (
    [switch]$All,
    [switch]$Stip,
    [switch]$Mock,
    [switch]$Daemon,
    [switch]$Ui,
    [switch]$ControlPlane,
    [switch]$OpsUi
)

# If no flags are provided, assume "All"
if (-not $Stip -and -not $Mock -and -not $Daemon -and -not $Ui -and -not $ControlPlane -and -not $OpsUi) {
    $All = $true
}

$EngineDir = "$PSScriptRoot\ISO8583PaymentEngine"
$UiDir = "$PSScriptRoot\ISO8583PaymentSimulator"
$ControlPlaneDir = "$PSScriptRoot\ops-control-plane"
$OpsUiDir = "$PSScriptRoot\ops-ui"

Write-Host "=========================================" -ForegroundColor Magenta
Write-Host "   ISO8583 Payment Engine Orchestrator   " -ForegroundColor Magenta
Write-Host "=========================================" -ForegroundColor Magenta

$global:runningProcs = @()

# Automatically hunt down and terminate any stale zombie services from prior runs
$targets = @("mock-bank-node", "payment-daemon", "mock-load-generator", "ops-control-plane", "mock_hsm")
$stale = Get-Process | Where-Object { $targets -contains $_.ProcessName }
if ($stale) {
    Write-Host "[!] Discovered previous engine services already running! Cancelling them..." -ForegroundColor Yellow
    $stale | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

function Clean-Up {
    Write-Host "`n[!] Aborting Sequence: Rolling back started services..." -ForegroundColor Red
    foreach ($p in $global:runningProcs) {
        if (-not $p.HasExited) {
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            Write-Host "   Killed orphaned process PID: $($p.Id)" -ForegroundColor DarkGray
        }
    }
    exit 1
}

function Boot-Service {
    param (
        [string]$TaskName,
        [string]$ProcessExe,
        [string]$CommandArgs,
        [string]$WorkingDir
    )
    Write-Host "-> $TaskName" -ForegroundColor Cyan
    $proc = Start-Process $ProcessExe -ArgumentList $CommandArgs -WorkingDirectory $WorkingDir -PassThru
    $global:runningProcs += $proc

    # Poll for 4 seconds to catch immediate startup crashes
    for ($i = 0; $i -lt 4; $i++) {
        Start-Sleep -Seconds 1
        if ($proc.HasExited) {
            Write-Host "   [FAILED] Service crashed immediately! (Exit Code: $($proc.ExitCode))" -ForegroundColor Red
            Clean-Up
        }
    }
    Write-Host "   [OK] Service stabilized." -ForegroundColor Green
}


if ($All -or $ControlPlane) {
    Write-Host "-> Booting Control Plane API (SQLite Initialized Natively)..." -ForegroundColor Cyan
    Boot-Service -TaskName "Booting Control Plane API (Port 3001)" -ProcessExe "cargo" -CommandArgs "run" -WorkingDir $ControlPlaneDir
}

if ($All -or $Stip) {
    Write-Host "-> Populating offline limits... (Service A: STIP Loader)" -ForegroundColor Cyan
    $stipProc = Start-Process "cargo" -ArgumentList "run --bin stip_loader" -WorkingDirectory $EngineDir -Wait -PassThru
    if ($stipProc.ExitCode -ne 0) {
        Write-Host "   [FAILED] STIP Loader experienced a fatal error." -ForegroundColor Red
        Clean-Up
    }
    Write-Host "   [OK] STIP Database securely populated." -ForegroundColor Green
}

if ($All -or $Mock) {
    Boot-Service -TaskName "Booting up Mock HSM..." -ProcessExe "cargo" -CommandArgs "run --bin mock_hsm" -WorkingDir $UiDir
    Boot-Service -TaskName "Booting up External Network Mocks... (Service B: Mock Bank Node)" -ProcessExe "cargo" -CommandArgs "run -p mock-bank-node" -WorkingDir $UiDir
}

if ($All -or $Daemon) {
    Boot-Service -TaskName "Launching main gateway... (Service C: Payment Daemon)" -ProcessExe "cargo" -CommandArgs "run --bin payment-daemon" -WorkingDir $EngineDir
}

if ($All -or $Ui) {
    Boot-Service -TaskName "Spinning up the Fast-Load Generator..." -ProcessExe "cargo" -CommandArgs "run --release --bin mock-load-generator" -WorkingDir $UiDir
}

if ($All -or $OpsUi) {
    Boot-Service -TaskName "Spinning up the Operations Dashboard (Port 3002)" -ProcessExe "npm.cmd" -CommandArgs "run dev -- -p 3002" -WorkingDir $OpsUiDir
}

if ($All -or $Daemon -or $ControlPlane) {
    Write-Host "-> Pushing Temporal Configurations into Switch Memory..." -ForegroundColor Cyan
    # Poll until Control Plane comes online and pushes config to Daemon
    for ($i = 0; $i -lt 15; $i++) {
        try {
            Invoke-RestMethod -Uri http://127.0.0.1:3003/api/publish/crdb -Method Post -ErrorAction Stop | Out-Null
            Invoke-RestMethod -Uri http://127.0.0.1:3003/api/publish/stip -Method Post -ErrorAction Stop | Out-Null
            Invoke-RestMethod -Uri http://127.0.0.1:3003/api/publish/routes -Method Post -ErrorAction Stop | Out-Null
            Write-Host "   [OK] Radix Trie State Pushed Natively!" -ForegroundColor Green
            break
        } catch {
            Start-Sleep -Seconds 2
        }
    }
}

Write-Host "`nAll targeted services have been verified and dispatched successfully!" -ForegroundColor Green
Write-Host "Tip: To selectively boot Domain 2 services, use flags like:"
Write-Host "  .\start_services.ps1 -Daemon -ControlPlane -OpsUi"
