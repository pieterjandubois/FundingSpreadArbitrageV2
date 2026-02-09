# Monitor trading-monitor output and log changes over time
$logFile = "trade_monitor.log"
$iteration = 0
$previousSnapshot = @{}

Write-Host "Starting trade monitoring... Logging to $logFile"
Write-Host ""

while ($true) {
    $iteration++
    $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff"
    
    # Get portfolio summary from Redis
    $portfolioJson = redis-cli GET "strategy:portfolio:state" 2>$null
    
    if ($portfolioJson) {
        $portfolio = $portfolioJson | ConvertFrom-Json
        
        # Extract key metrics
        $activeTrades = $portfolio.active_trades.Count
        $closedTrades = $portfolio.closed_trades.Count
        $availableCapital = [Math]::Round($portfolio.available_capital, 2)
        $totalOpen = [Math]::Round($portfolio.total_open_positions, 2)
        $startingCapital = $portfolio.starting_capital
        
        # Get metrics
        $metricsJson = redis-cli GET "strategy:portfolio:metrics" 2>$null
        $metrics = if ($metricsJson) { $metricsJson | ConvertFrom-Json } else { $null }
        
        $cumulativePnL = if ($metrics) { [Math]::Round($metrics.cumulative_pnl, 2) } else { 0 }
        $winRate = if ($metrics) { [Math]::Round($metrics.win_rate, 1) } else { 0 }
        $totalTrades = if ($metrics) { $metrics.total_trades } else { 0 }
        
        # Log snapshot
        $output = "[$timestamp] Iter:$iteration | Active:$activeTrades | Closed:$closedTrades | Available:$availableCapital | Open:$totalOpen | PnL:$cumulativePnL | WinRate:$winRate%"
        
        Write-Host $output
        Add-Content -Path $logFile -Value $output
        
        # Check for changes
        if ($previousSnapshot.activeTrades -ne $activeTrades) {
            $msg = "  CHANGE: Active trades: $($previousSnapshot.activeTrades) -> $activeTrades"
            Write-Host $msg -ForegroundColor Yellow
            Add-Content -Path $logFile -Value $msg
        }
        if ($previousSnapshot.closedTrades -ne $closedTrades) {
            $msg = "  CHANGE: Closed trades: $($previousSnapshot.closedTrades) -> $closedTrades"
            Write-Host $msg -ForegroundColor Yellow
            Add-Content -Path $logFile -Value $msg
        }
        if ([Math]::Abs($previousSnapshot.availableCapital - $availableCapital) -gt 1) {
            $msg = "  CHANGE: Available capital: $($previousSnapshot.availableCapital) -> $availableCapital"
            Write-Host $msg -ForegroundColor Yellow
            Add-Content -Path $logFile -Value $msg
        }
        if ([Math]::Abs($previousSnapshot.cumulativePnL - $cumulativePnL) -gt 0.01) {
            $msg = "  CHANGE: Cumulative PnL: $($previousSnapshot.cumulativePnL) -> $cumulativePnL"
            Write-Host $msg -ForegroundColor Yellow
            Add-Content -Path $logFile -Value $msg
        }
        
        # Check for anomalies
        if ($activeTrades -gt 50 -and $availableCapital -lt 100) {
            $msg = "  WARNING: Many active trades ($activeTrades) with very low capital ($availableCapital)"
            Write-Host $msg -ForegroundColor Red
            Add-Content -Path $logFile -Value $msg
        }
        
        if ($closedTrades -gt 100 -and $winRate -lt 50) {
            $msg = "  WARNING: Low win rate ($winRate%) with $closedTrades closed trades"
            Write-Host $msg -ForegroundColor Red
            Add-Content -Path $logFile -Value $msg
        }
        
        $previousSnapshot = @{
            activeTrades = $activeTrades
            closedTrades = $closedTrades
            availableCapital = $availableCapital
            totalOpen = $totalOpen
            cumulativePnL = $cumulativePnL
            winRate = $winRate
        }
    }
    
    Start-Sleep -Seconds 5
}
