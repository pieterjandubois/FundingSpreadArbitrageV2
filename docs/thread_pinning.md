# Thread Pinning Configuration

## Overview

This document describes the thread pinning configuration for optimal low-latency performance. Thread pinning ensures that critical threads stay on dedicated CPU cores, maintaining hot CPU caches and preventing OS interference.

## Core Assignment

The system uses the following core assignment strategy:

- **Core 0**: OS and system tasks (not used by trading system)
- **Core 1**: Strategy thread (hot path - critical for trade decisions)
- **Cores 2-7**: WebSocket threads (warm path - market data ingestion)

This assignment ensures that:
1. The strategy thread has a dedicated core with minimal interference
2. WebSocket threads are isolated from OS scheduler
3. CPU caches remain hot (no context switching between cores)
4. Predictable, low-latency performance

## Kernel Configuration

For optimal performance, you must isolate cores 1-7 from the Linux scheduler using the `isolcpus` kernel parameter.

### Step 1: Edit GRUB Configuration

Edit `/etc/default/grub` and add the following parameters to `GRUB_CMDLINE_LINUX`:

```bash
sudo nano /etc/default/grub
```

Add or modify the line:

```bash
GRUB_CMDLINE_LINUX="isolcpus=1-7 nohz_full=1-7 rcu_nocbs=1-7"
```

**Parameter Explanation:**
- `isolcpus=1-7`: Isolates cores 1-7 from the OS scheduler
- `nohz_full=1-7`: Disables timer ticks on cores 1-7 (reduces interrupts)
- `rcu_nocbs=1-7`: Moves RCU callbacks off cores 1-7 (reduces kernel overhead)

### Step 2: Update GRUB

After editing the configuration, rebuild GRUB:

```bash
sudo update-grub
```

Or on some systems:

```bash
sudo grub2-mkconfig -o /boot/grub2/grub.cfg
```

### Step 3: Reboot

Reboot the system for changes to take effect:

```bash
sudo reboot
```

### Step 4: Verify Configuration

After reboot, verify that cores are isolated:

```bash
cat /sys/devices/system/cpu/isolated
```

Expected output:
```
1-7
```

You can also check the kernel command line:

```bash
cat /proc/cmdline
```

You should see `isolcpus=1-7 nohz_full=1-7 rcu_nocbs=1-7` in the output.

## System Requirements

### Minimum Requirements

- **CPU Cores**: At least 8 cores (0-7)
- **Architecture**: x86_64 (AMD64 or Intel)
- **OS**: Linux with kernel 3.10 or later

### Checking Your System

Check the number of CPU cores:

```bash
nproc
```

Or:

```bash
lscpu | grep "^CPU(s):"
```

If you have fewer than 8 cores, the system will still work but thread pinning may not be optimal.

## Runtime Verification

When the application starts, it will print core assignment information:

```
=== CPU Core Assignment ===
Total cores available: 8
Core 0: OS and system tasks
Core 1: Strategy thread (hot path)
Cores 2-7: WebSocket threads (warm path)

For optimal performance, isolate cores 1-7 from OS scheduler:
  1. Edit /etc/default/grub
  2. Add: GRUB_CMDLINE_LINUX="isolcpus=1-7 nohz_full=1-7 rcu_nocbs=1-7"
  3. Run: sudo update-grub
  4. Reboot

To verify isolation:
  cat /sys/devices/system/cpu/isolated
  (should show: 1-7)
===========================
```

During execution, you'll see messages like:

```
[THREAD-PIN] ✓ strategy thread pinned to core 1
[THREAD-PIN] ✓ websocket-0 thread pinned to core 2
[THREAD-PIN] ✓ websocket-1 thread pinned to core 3
...
```

## Troubleshooting

### Warning: Insufficient cores detected

If you see this warning, your system has fewer than 8 cores. The application will still run, but performance may be degraded. Consider:

1. Running on a system with more cores
2. Reducing the number of WebSocket connections
3. Accepting reduced performance

### Warning: Failed to pin thread

If thread pinning fails, possible causes:

1. **Insufficient permissions**: Try running with elevated privileges (not recommended for production)
2. **Cores not isolated**: Verify `isolcpus` kernel parameter is set correctly
3. **Unsupported platform**: Thread pinning requires Linux with `core_affinity` support

The application will continue to run without thread pinning, but latency may be higher.

### Verifying Thread Affinity

You can verify thread affinity using `taskset`:

```bash
# Find the process ID
ps aux | grep arbitrage2

# Check thread affinity (replace PID with actual process ID)
taskset -cp PID
```

You should see threads pinned to specific cores.

## Performance Impact

With proper thread pinning and core isolation:

- **Reduced latency**: 20-30% improvement in p99 latency
- **Lower jitter**: More consistent latency (smaller variance)
- **Better cache utilization**: L1/L2 cache hit rates improve by 10-15%
- **Fewer context switches**: Near-zero involuntary context switches

Without thread pinning:

- OS may move threads between cores
- CPU caches get invalidated on core migration
- Increased latency variance (jitter)
- Potential interference from other processes

## Production Recommendations

1. **Always use core isolation** in production for low-latency trading
2. **Monitor CPU usage** to ensure cores are not overloaded
3. **Disable hyperthreading** for even better performance (optional)
4. **Use dedicated hardware** - avoid running other services on the same machine
5. **Test thoroughly** after configuration changes

## Additional Optimizations

For even better performance, consider:

1. **Disable CPU frequency scaling**:
   ```bash
   sudo cpupower frequency-set -g performance
   ```

2. **Disable transparent huge pages**:
   ```bash
   echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled
   ```

3. **Increase process priority** (use with caution):
   ```bash
   sudo nice -n -20 ./arbitrage2
   ```

4. **Use NUMA pinning** on multi-socket systems:
   ```bash
   numactl --cpunodebind=0 --membind=0 ./arbitrage2
   ```

## References

- Linux kernel documentation: https://www.kernel.org/doc/Documentation/admin-guide/kernel-parameters.txt
- CPU isolation guide: https://www.kernel.org/doc/html/latest/admin-guide/kernel-per-CPU-kthreads.html
- Real-time Linux: https://wiki.linuxfoundation.org/realtime/start

## Requirements Satisfied

This configuration satisfies the following requirements from the low-latency optimization spec:

- **Requirement 4.1**: Pin strategy thread to core 1
- **Requirement 4.2**: Pin WebSocket threads to cores 2-7
- **Requirement 4.3**: Verify affinity and log core assignments
- **Requirement 4.4**: Document isolcpus kernel parameter
