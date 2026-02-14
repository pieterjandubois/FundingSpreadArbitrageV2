# scripts/run_benchmarks.ps1
# Run performance benchmarks and compare against baseline (PowerShell version)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$BaselineFile = Join-Path $ProjectRoot "baseline_metrics.json"
$ResultsFile = Join-Path $ProjectRoot "benchmark_results.txt"
$MetricsFile = Join-Path $ProjectRoot "current_metrics.json"

Write-Host "=================================================="
Write-Host "Performance Benchmark Suite"
Write-Host "=================================================="
Write-Host ""

# Build in release mode
Write-Host "Building release binary..."
$env:RUSTFLAGS = "-C target-cpu=native -C opt-level=3"
cargo build --release

Write-Host ""
Write-Host "Running benchmarks..."
Write-Host "This may take several minutes..."
Write-Host ""

# Run benchmarks and capture output
cargo test --release -- --ignored --nocapture --test-threads=1 2>&1 | Tee-Object -FilePath $ResultsFile

Write-Host ""
Write-Host "Parsing benchmark results..."

# Function to extract P99 from benchmark output
function Extract-P99 {
    param([string]$BenchmarkName)
    
    $content = Get-Content $ResultsFile -Raw
    $pattern = "Benchmark: $BenchmarkName[\s\S]*?P99:\s+(\d+)"
    
    if ($content -match $pattern) {
        return [int]$Matches[1]
    }
    return 0
}

# Function to extract Average from benchmark output
function Extract-Avg {
    param([string]$BenchmarkName)
    
    $content = Get-Content $ResultsFile -Raw
    $pattern = "Benchmark: $BenchmarkName[\s\S]*?Average:\s+(\d+)"
    
    if ($content -match $pattern) {
        return [int]$Matches[1]
    }
    return 0
}

# Extract metrics for key benchmarks
$marketUpdateP99 = Extract-P99 "market_data_update"
$marketUpdateAvg = Extract-Avg "market_data_update"

$spreadCalcP99 = Extract-P99 "market_data_spread_calculation"
$spreadCalcAvg = Extract-Avg "market_data_spread_calculation"

$branchlessValidationP99 = Extract-P99 "branchless_opportunity_validation"
$branchlessValidationAvg = Extract-Avg "branchless_opportunity_validation"

$sequentialAccessP99 = Extract-P99 "market_data_sequential_iteration"
$randomAccessP99 = Extract-P99 "market_data_random_access"

# Save current metrics
$metrics = @{
    market_update_p99_ns = $marketUpdateP99
    market_update_avg_ns = $marketUpdateAvg
    spread_calc_p99_ns = $spreadCalcP99
    spread_calc_avg_ns = $spreadCalcAvg
    branchless_validation_p99_ns = $branchlessValidationP99
    branchless_validation_avg_ns = $branchlessValidationAvg
    sequential_access_p99_ns = $sequentialAccessP99
    random_access_p99_ns = $randomAccessP99
}

$metrics | ConvertTo-Json | Set-Content $MetricsFile

Write-Host ""
Write-Host "Current Metrics:"
Get-Content $MetricsFile

# Compare against baseline if it exists
if (Test-Path $BaselineFile) {
    Write-Host ""
    Write-Host "=================================================="
    Write-Host "Comparing Against Baseline"
    Write-Host "=================================================="
    
    $current = Get-Content $MetricsFile | ConvertFrom-Json
    $baseline = Get-Content $BaselineFile | ConvertFrom-Json
    
    $regressions = @()
    $improvements = @()
    $stable = @()
    
    foreach ($prop in $current.PSObject.Properties) {
        $metricName = $prop.Name
        $currentVal = $prop.Value
        
        if (-not $baseline.PSObject.Properties[$metricName]) {
            continue
        }
        
        $baselineVal = $baseline.PSObject.Properties[$metricName].Value
        
        if ($currentVal -eq 0 -or $baselineVal -eq 0) {
            continue
        }
        
        $changePct = (($currentVal - $baselineVal) / $baselineVal) * 100
        
        $result = @{
            metric = $metricName
            baseline = $baselineVal
            current = $currentVal
            change_pct = $changePct
        }
        
        if ($changePct -gt 10) {
            $regressions += $result
        } elseif ($changePct -lt -10) {
            $improvements += $result
        } else {
            $stable += $result
        }
    }
    
    # Print results
    Write-Host ""
    Write-Host "============================================================"
    Write-Host "BENCHMARK COMPARISON RESULTS"
    Write-Host "============================================================"
    
    if ($regressions.Count -gt 0) {
        Write-Host ""
        Write-Host "❌ REGRESSIONS ($($regressions.Count)):" -ForegroundColor Red
        foreach ($reg in $regressions) {
            Write-Host "  $($reg.metric):"
            Write-Host "    Baseline: $($reg.baseline) ns"
            Write-Host "    Current:  $($reg.current) ns"
            Write-Host "    Change:   $($reg.change_pct.ToString('+0.00;-0.00'))% (SLOWER)" -ForegroundColor Red
        }
    }
    
    if ($improvements.Count -gt 0) {
        Write-Host ""
        Write-Host "✅ IMPROVEMENTS ($($improvements.Count)):" -ForegroundColor Green
        foreach ($imp in $improvements) {
            Write-Host "  $($imp.metric):"
            Write-Host "    Baseline: $($imp.baseline) ns"
            Write-Host "    Current:  $($imp.current) ns"
            Write-Host "    Change:   $($imp.change_pct.ToString('+0.00;-0.00'))% (FASTER)" -ForegroundColor Green
        }
    }
    
    if ($stable.Count -gt 0) {
        Write-Host ""
        Write-Host "✓ STABLE ($($stable.Count)):"
        foreach ($s in $stable) {
            Write-Host "  $($s.metric): $($s.change_pct.ToString('+0.00;-0.00'))%"
        }
    }
    
    Write-Host ""
    Write-Host "============================================================"
    
    if ($regressions.Count -gt 0) {
        Write-Host "❌ FAILED: Performance regressions detected" -ForegroundColor Red
        Write-Host ""
        Write-Host "⚠️  Performance regression detected!"
        Write-Host "Review the changes and optimize before committing."
        exit 1
    } else {
        Write-Host "✅ PASSED: No performance regressions" -ForegroundColor Green
    }
} else {
    Write-Host ""
    Write-Host "No baseline found. Creating baseline from current metrics..."
    Copy-Item $MetricsFile $BaselineFile
    Write-Host "✓ Baseline created: $BaselineFile"
    Write-Host ""
    Write-Host "Run this script again to compare future changes against this baseline."
}

Write-Host ""
Write-Host "=================================================="
Write-Host "Benchmark Complete"
Write-Host "=================================================="
Write-Host ""
Write-Host "Results saved to: $ResultsFile"
Write-Host "Metrics saved to: $MetricsFile"
Write-Host "Baseline: $BaselineFile"
Write-Host ""
Write-Host "To view full results:"
Write-Host "  Get-Content $ResultsFile"
Write-Host ""
Write-Host "To update baseline:"
Write-Host "  Copy-Item $MetricsFile $BaselineFile"
Write-Host ""
