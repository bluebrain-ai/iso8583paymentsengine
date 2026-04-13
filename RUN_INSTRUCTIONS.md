# Comprehensive Operations & Runbook

This document details every service required for the ISO8583 Payment Engine, how to execute them, how to monitor their health, and how to rigorously test them.

---

## 1. Services to Start

The architecture relies on multiple isolated physical binaries and processes to guarantee memory isolation and Staged Event-Driven Architecture (SEDA) safety.

### Service A: The STIP Loader (Stand-In Processing Database)
* **Description:** Pre-populates your embedded `sled` offline database. If the network goes down, the daemon uses this to authorize/decline transactions locally without network bounds.
* **Execution:**
  ```powershell
  cd d:\cob2java\GitHub\ISOPaymentEngine\ISO8583PaymentEngine
  cargo run --bin stip_loader
  ```
* **Output:** Generates/Updates the `stip.db` file in the directory.

### Service B: Mock Bank Mainframe Simulator
* **Description:** An isolated TCP daemon simulating the high-latency network behavior of institutional bank networks (Visa/Mastercard). It listens for pure ISO8583 hex, applies a network delay, and generates outbound `0110` canonical approvals.
  - Binds Visa TCP Socket: `127.0.0.1:9188`
  - Binds Mastercard TCP Socket: `127.0.0.1:9200`
* **Execution:**
  ```powershell
  cd d:\cob2java\GitHub\ISOPaymentEngine\ISO8583PaymentEngine
  cargo run --bin mock-bank-node
  ```

### Service C: The Core Payment Daemon
* **Description:** The master multi-threaded executable. It houses the Zero-Copy parser, the `memmap2` Journal/WAL, Idris DashMap for Idempotency, and the embedded **Axum Telemetry API**. It routes TCP ingress directly over asynchronous sockets out to Service B.
  - Binds raw TCP ingest socket: `127.0.0.1:8000`
  - Binds JSON Telemetry API: `0.0.0.0:8080`
* **Execution (Open a new terminal):**
  ```powershell
  cd d:\cob2java\GitHub\ISOPaymentEngine\ISO8583PaymentEngine
  cargo run --bin payment-daemon
  ```

### Service D: The React Telemetry Dashboard & Simulator
* **Description:** The browser-based visualizer and payment simulator. Its internal Vite server automatically proxies web `POST` requests directly into raw TCP bytes for Service B.
* **Execution (Open a new terminal):**
  ```powershell
  cd d:\cob2java\GitHub\ISOPaymentEngine\ISO8583PaymentSimulator
  npm install
  npm run dev
  ```
nvm 
---

## 2. Healthchecks

### Monitoring Payment Daemon Health (Rust)
Because the daemon leverages native Tokio listeners and SEDA pipelines, its health is binary and observed via network boundaries:
1. **TCP Port Check:** Ensure the `edge_ingress` is running natively by querying your open ports.
   ```powershell
   Test-NetConnection -ComputerName localhost -Port 8000
   ```
   *Expected Output: `TcpTestSucceeded : True`*
2. **Telemetry API (Axum):** Query the backend's memory bound via terminal to pull the live Dashmap array limits:
   ```powershell
   curl http://localhost:8080/api/dashmap
   ```
3. **Prometheus Metrics:** Check your environment for the collector exposing port `9090`:
   ```powershell
   curl http://localhost:9090/metrics | Select-String "transactions_total"
   ```

### Monitoring React Simulator Health (Vite)
1. **UI Availability:** Open `http://localhost:5173/` in a web browser.
2. **Vite Proxy Logs:** The terminal running `npm run dev` will output error codes (e.g. `502 TCP Backend Error` or `504 Timeout`) if Service C is offline and the Vite Proxy cannot bind to `8000`.

---

## 3. Retrieving Transaction Status from Logs

The SEDA framework ensures that every transaction is tracked persistently. You can trace outcomes using three distinct methods:

### Method A: Rust `stdout` Traces (Real-time Live Stream)
The `payment-daemon` uses the `tracing` crate. When live transactions run, the primary `payment-daemon` terminal will log sequences like:
> `[INFO] atm_connection peer_addr="127.0.0.1:54932"`
> `[INFO] STIP Fallback Authorized amount=...`  *(In the event of a simulated disconnected mainframe)*

### Method B: The WAL (Write-Ahead Log)
For forensics and auditing, the SEDA architecture immediately persists every request to the physical disk **before** routing it to the core.
* **Location:** `d:\cob2java\GitHub\ISOPaymentEngine\ISO8583PaymentEngine\live_payment_wal.log`
* **How to read:** This log is extremely fast and binary-heavy using `memmap2`. To view human-readable dumps, you can observe file growth or parse chunk sizes.

### Method C: The Live Telemetry Dashboard
When generating traffic through the UI, use the **Live Engine Introspection** tab and **Message Inspector** grid at the bottom of the page. They map:
* Real-time physical active memory lock levels inside the Rust `DashMap`.
* Real-time WAL transaction deserializations.
* **Trace ID** (`TRACE-1678...`) and Final **Status** (`Success`, `Timeout`, `Decline`).

---

## 4. How to Test the Environment

### Approach 1: Using the Visual UI Simulator
1. Open up the React Dashboard at `http://localhost:5173`.
2. Under the **Control Center** widget, select your distribution percentage (e.g. 70% Happy Path, 20% Decline, 10% Timeout) and Target Endpoint.
3. Set volume to **100** transactions or more.
4. Click **Deploy Load Test**. 
5. Need to stop early? Hit the **Red (X)** Cancel button situated next to the executing spinner. Our UI implements a physical circuit breaker that terminates loop executions instantly.
6. The graphical map will pulse dynamically based on what the Rust backend resolves! The Latency charting will spike naturally based on `DashMap` lock contentions, and the `Live Introspection` monitor will show raw memory limits mapping live.

### Approach 2: Manually via Raw TCP Sockets (Hardcore Terminal Mode)
To bypass the Vite middleman and act as a literal Hardware ATM, you can blast pure UTF-8 ISO bytes into the daemon's TCP socket.

**Using Netcat (if installed) or PowerShell's TcpClient:**

Run the following command in PowerShell to instantly fire a single raw ISO-8583 payload byte-string directly into the memory boundary of the Engine.

```powershell
$socket = New-Object System.Net.Sockets.TcpClient("127.0.0.1", 8000)
$stream = $socket.GetStream()
$writer = New-Object System.IO.StreamWriter($stream)

# Our RAW Mock ISO8583 HEX Packet (Message Type: 0100, PAN mapping simulated)
$iso_packet = "0100322000000080800000000000000000000TESTPAYLOAD12345"

$writer.Write($iso_packet)
$writer.Flush()

# Wait for Rust to drop standard Reply (0110 Approval or Timeout)
$reader = New-Object System.IO.StreamReader($stream)
$response = $reader.ReadLine()
Write-Host "RUST ENGINE REPONSE: $response"

$socket.Close()
```
If the backend accepts it natively, it will parse into the zero-copy buffer and emit trace logs in the primary window!
