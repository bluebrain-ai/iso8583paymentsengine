$body = @{
    volume = 100000
    concurrency = 1500
} | ConvertTo-Json

try {
    Invoke-RestMethod -Uri "http://localhost:3001/sim/start" -Method Post -Body $body -ContentType "application/json" | Out-Null
    Write-Host "Started Test!" -ForegroundColor Green
} catch {
    Write-Host "Failed to start. Simulation might already be running." -ForegroundColor Yellow
}

Start-Sleep -Seconds 2

while ($true) {
    try {
        $status = Invoke-RestMethod -Uri "http://localhost:3001/sim/status" -Method Get
        if (-not $status.running) {
            Write-Host "Test completed!" -ForegroundColor Green
            break
        }
        $comp = $status.completed
        $tps = $status.tps
        $lat = $status.avgLatencyMs
        Write-Host "Completed: $comp | TPS: $tps | Latency: ${lat}ms" 
        Start-Sleep -Seconds 1
    } catch {
        Write-Host "Error fetching status..."
        Start-Sleep -Seconds 1
    }
}
