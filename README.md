# Antigravity ISO-8583 Payment Switch Engine

A high-throughput, latency-optimized Payment Engine written natively in Rust, architected to gracefully process over 10,000 ISO Transactions per second simultaneously via symmetric Staged Event-Driven Architectures (SEDA), `dashmap` tracking, and embedded `sled` DB components.

## Architecture Deep Dives & Lessons Learned

### SEDA Thread Pool Symmetry & OS Context Overhead
When tuning the core Payment Engine (`edge-ingress`, `switch-core`, `edge-egress`) for absolute maximum concurrent throughput, extreme care must be taken with the `std::thread::spawn` invocation patterns.

Because the core processing loop guarantees durability via Write-Ahead Logs (WAL) attached to an embedded Sled Database via `journaler.append()`, it is performing synchronous Disk I/O.
- We deliberately use raw operating system threads (`std::thread::spawn`) rather than Tokio’s async `tokio::spawn` loops internally in `switch-core` to guarantee this synchronous disk blocking cannot randomly freeze the Tokio asynchronous TCP reactors processing live payments.
- We restrict the `ingress_rx` and `bank_reply_rx` internal queues tightly to **exactly 8 CPU cores**, operating harmonically parallel. 
- Deploying unbounded thread mappings (e.g., launching 50-200 raw OS threads) against `sled` lock borders results in catastrophic computational CPU "Context Switching." The operating system begins spending drastically more time rapidly tearing down and storing localized memory constraints between raw threads over the CPU cores than organically processing bitstreams. 

`For future Mainframe scalability:` Replace `0..8` with `0..num_cpus::get()` to universally bind the worker concurrency boundaries to identical harmonic matches alongside underlying computational arrays.

### Simulator Load Generation: Overcoming Ephemeral TIME_WAIT Port Exhaustion 
The Rust native `axum` based Simulator (`mock-load-generator`) pushes up to 100,000 asynchronous transactions.
Earlier load testing resulted in immediate and massive timeout waves (<300ms round trip responses registering as Timeout "68"). Investigation revealed 500 parallel workers allocating raw Windows Ephemeral Target Ports individually on a `1:1` ratio against discrete TCP instances (`TcpStream::connect`), opening and abruptly closing 100,000 sequential channels.
- `Root Cause`: Rapidly dropping active TCP stream pipelines without graceful connection pooling immediately overwhelms the host server's OS ephemeral bounds (~16,384 available ports limits on Windows). Sockets fall into zombie OS `TIME_WAIT` persistence for 3-5 seconds after closure, starving the network stack. 
- `Current Baseline`: The Simulator relies on asynchronous HTTP Connection Pooling algorithms equivalent to production load drivers (`JMeter`, `k6`)—natively sustaining distinct `tokio` frames natively persisted across consecutive payloads in its pipeline loop. Zero timeouts occur at maximum concurrent capacity profiles.

For localized orchestration, refer to `start_services.ps1` configurations located in the parent directory!
