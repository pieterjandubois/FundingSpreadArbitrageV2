# build-release.ps1
# Optimized release build script for low-latency trading system (Windows)

Write-Host "Building with maximum optimizations..." -ForegroundColor Green

# Set RUSTFLAGS for native CPU optimizations
$env:RUSTFLAGS = "-C target-cpu=native -C opt-level=3"

# Build release binary
cargo build --release

if ($LASTEXITCODE -eq 0) {
    Write-Host "`nBuild complete!" -ForegroundColor Green
    Write-Host ""
    Write-Host "Running clippy checks..." -ForegroundColor Yellow
    cargo clippy --release -- -D warnings
    
    Write-Host ""
    Write-Host "Binary location: target\release\arbitrage2.exe" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "To install flamegraph profiler (first time only):" -ForegroundColor Cyan
    Write-Host "  cargo install flamegraph" -ForegroundColor White
    Write-Host ""
    Write-Host "To profile with flamegraph, run:" -ForegroundColor Cyan
    Write-Host "  cargo flamegraph --release --bin arbitrage2" -ForegroundColor White
} else {
    Write-Host "`nBuild failed!" -ForegroundColor Red
    exit 1
}
