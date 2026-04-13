Start-Process "cargo" -ArgumentList "run --bin stip_loader" -WorkingDirectory "$PSScriptRoot\ISO8583PaymentEngine" -Wait
Start-Process "cargo" -ArgumentList "run -p mock-bank-node" -WorkingDirectory "$PSScriptRoot\ISO8583PaymentSimulator" -RedirectStandardOutput bank.log -RedirectStandardError bank.err -WindowStyle Hidden
Start-Process "cargo" -ArgumentList "run --bin payment-daemon" -WorkingDirectory "$PSScriptRoot\ISO8583PaymentEngine" -RedirectStandardOutput daemon.log -RedirectStandardError daemon.err -WindowStyle Hidden
Start-Process "cargo" -ArgumentList "run --release --bin mock-load-generator" -WorkingDirectory "$PSScriptRoot\ISO8583PaymentSimulator" -RedirectStandardOutput load.log -RedirectStandardError load.err -WindowStyle Hidden
Write-Host "Services started inline!"
