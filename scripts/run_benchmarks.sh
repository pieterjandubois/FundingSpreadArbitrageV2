#!/bin/bash
# scripts/run_benchmarks.sh
# Run performance benchmarks and compare against baseline

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BASELINE_FILE="$PROJECT_ROOT/baseline_metrics.json"
RESULTS_FILE="$PROJECT_ROOT/benchmark_results.txt"
METRICS_FILE="$PROJECT_ROOT/current_metrics.json"

echo "=================================================="
echo "Performance Benchmark Suite"
echo "=================================================="
echo ""

# Build in release mode
echo "Building release binary..."
export RUSTFLAGS="-C target-cpu=native -C opt-level=3"
cargo build --release

echo ""
echo "Running benchmarks..."
echo "This may take several minutes..."
echo ""

# Run benchmarks and capture output
cargo test --release -- --ignored --nocapture --test-threads=1 > "$RESULTS_FILE" 2>&1 || true

echo ""
echo "Parsing benchmark results..."

# Extract key metrics
extract_p99() {
  local benchmark_name="$1"
  local p99=$(grep -A 10 "Benchmark: $benchmark_name" "$RESULTS_FILE" | grep "P99:" | awk '{print $2}' | head -1)
  echo "${p99:-0}"
}

extract_avg() {
  local benchmark_name="$1"
  local avg=$(grep -A 10 "Benchmark: $benchmark_name" "$RESULTS_FILE" | grep "Average:" | awk '{print $2}' | head -1)
  echo "${avg:-0}"
}

# Extract metrics for key benchmarks
market_update_p99=$(extract_p99 "market_data_update")
market_update_avg=$(extract_avg "market_data_update")

spread_calc_p99=$(extract_p99 "market_data_spread_calculation")
spread_calc_avg=$(extract_avg "market_data_spread_calculation")

branchless_validation_p99=$(extract_p99 "branchless_opportunity_validation")
branchless_validation_avg=$(extract_avg "branchless_opportunity_validation")

sequential_access_p99=$(extract_p99 "market_data_sequential_iteration")
random_access_p99=$(extract_p99 "market_data_random_access")

# Save current metrics
cat > "$METRICS_FILE" << EOF
{
  "market_update_p99_ns": ${market_update_p99},
  "market_update_avg_ns": ${market_update_avg},
  "spread_calc_p99_ns": ${spread_calc_p99},
  "spread_calc_avg_ns": ${spread_calc_avg},
  "branchless_validation_p99_ns": ${branchless_validation_p99},
  "branchless_validation_avg_ns": ${branchless_validation_avg},
  "sequential_access_p99_ns": ${sequential_access_p99},
  "random_access_p99_ns": ${random_access_p99}
}
EOF

echo ""
echo "Current Metrics:"
cat "$METRICS_FILE"

# Compare against baseline if it exists
if [ -f "$BASELINE_FILE" ]; then
  echo ""
  echo "=================================================="
  echo "Comparing Against Baseline"
  echo "=================================================="
  
  # Create Python comparison script
  python3 - << 'PYTHON_SCRIPT'
import json
import sys

# Load metrics
with open('current_metrics.json', 'r') as f:
    current = json.load(f)

with open('baseline_metrics.json', 'r') as f:
    baseline = json.load(f)

# Compare metrics
regressions = []
improvements = []
stable = []

for metric_name in current.keys():
    if metric_name not in baseline:
        continue
    
    current_val = current[metric_name]
    baseline_val = baseline[metric_name]
    
    if current_val == 0 or baseline_val == 0:
        continue
    
    change_pct = ((current_val - baseline_val) / baseline_val) * 100
    
    result = {
        'metric': metric_name,
        'baseline': baseline_val,
        'current': current_val,
        'change_pct': change_pct
    }
    
    if change_pct > 10:
        regressions.append(result)
    elif change_pct < -10:
        improvements.append(result)
    else:
        stable.append(result)

# Print results
print("\n" + "="*60)
print("BENCHMARK COMPARISON RESULTS")
print("="*60)

if regressions:
    print(f"\n❌ REGRESSIONS ({len(regressions)}):")
    for reg in regressions:
        print(f"  {reg['metric']}:")
        print(f"    Baseline: {reg['baseline']} ns")
        print(f"    Current:  {reg['current']} ns")
        print(f"    Change:   {reg['change_pct']:+.2f}% (SLOWER)")

if improvements:
    print(f"\n✅ IMPROVEMENTS ({len(improvements)}):")
    for imp in improvements:
        print(f"  {imp['metric']}:")
        print(f"    Baseline: {imp['baseline']} ns")
        print(f"    Current:  {imp['current']} ns")
        print(f"    Change:   {imp['change_pct']:+.2f}% (FASTER)")

if stable:
    print(f"\n✓ STABLE ({len(stable)}):")
    for s in stable:
        print(f"  {s['metric']}: {s['change_pct']:+.2f}%")

print("\n" + "="*60)

if regressions:
    print("❌ FAILED: Performance regressions detected")
    sys.exit(1)
else:
    print("✅ PASSED: No performance regressions")
    sys.exit(0)

PYTHON_SCRIPT

  COMPARISON_RESULT=$?
  
  if [ $COMPARISON_RESULT -ne 0 ]; then
    echo ""
    echo "⚠️  Performance regression detected!"
    echo "Review the changes and optimize before committing."
    exit 1
  fi
else
  echo ""
  echo "No baseline found. Creating baseline from current metrics..."
  cp "$METRICS_FILE" "$BASELINE_FILE"
  echo "✓ Baseline created: $BASELINE_FILE"
  echo ""
  echo "Run this script again to compare future changes against this baseline."
fi

echo ""
echo "=================================================="
echo "Benchmark Complete"
echo "=================================================="
echo ""
echo "Results saved to: $RESULTS_FILE"
echo "Metrics saved to: $METRICS_FILE"
echo "Baseline: $BASELINE_FILE"
echo ""
echo "To view full results:"
echo "  cat $RESULTS_FILE"
echo ""
echo "To update baseline:"
echo "  cp $METRICS_FILE $BASELINE_FILE"
echo ""
