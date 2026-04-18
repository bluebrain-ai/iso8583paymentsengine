Write-Host "Starting God Mode Payment Command Center..." -ForegroundColor Cyan

Write-Host "Starting Rust Test Orchestrator (Port 9000)..."
Start-Process -FilePath "cargo" -ArgumentList "run" -WorkingDirectory "d:\cob2java\GitHub\ISOPaymentEngine\test-orchestrator"

Write-Host "Starting Node.js BFF Gateway (Port 3000)..."
Start-Process -FilePath "npm" -ArgumentList "start" -WorkingDirectory "d:\cob2java\GitHub\ISOPaymentEngine\ops-bff"

Write-Host "Starting Next.js Unified Shell (Port 4000)..."
Start-Process -FilePath "npm" -ArgumentList "run dev" -WorkingDirectory "d:\cob2java\GitHub\ISOPaymentEngine\ops-ui"

Write-Host "All components launched in background windows." -ForegroundColor Green
