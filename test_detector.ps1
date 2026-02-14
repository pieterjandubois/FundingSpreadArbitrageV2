# Run bybit-synthetic-test for 30 seconds and capture output
$job = Start-Job -ScriptBlock {
    Set-Location $using:PWD
    & ".\target\release\bybit-synthetic-test.exe" 2>&1
}

# Wait 30 seconds
Start-Sleep -Seconds 30

# Stop the job
Stop-Job $job
Receive-Job $job | Select-String -Pattern "DETECTOR" | Select-Object -Last 50
Remove-Job $job
