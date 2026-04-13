# ISO 8583 Payment Engine - Testing Scripts and Context

This folder contains testing scripts, mock payloads, and utility architecture data to be used for executing End-to-End (E2E) testing against the Payment Switch Engine.

## Background Architecture Context
The Payment Switch operates over asynchronous TCP sockets separated into an **Ingress** layer (to receive authorization payloads from POS terminals/Acquirers) and an **Egress** layer (to route those payloads to Target Banks based on PAN mapping).

The routing algorithm internally leverages:
- **Zero-Copy Parser**: Expects the first 4 bytes to represent the MTI (`0100` Authorization Request).
- **Idempotency Guard**: Maps the `RRN + STAN + Acquirer ID` fields to enforce uniqueness. Duplicate TCP requests are intrinsically dropped.
- **Dynamic PAN Routing**: Reads the `PAN`. Identifiers starting with `4` are routed sequentially towards the VISA Egress channel, while those with `5` route to the MASTERCARD Egress Channel.

## Files Provided

### 1. `mock_ingress_payload.hex`
The raw representation of our test payload.
- **Prefix**: `00 0C` (12 bytes total network payload)
- **MTI**: `30 31 30 30` (ASCII representation for `0100`)
- **Bitmap**: `FF 00 00 00 00 00 00 00` (Mock primary bitmap)

### 2. `simulate_pos_terminal.py`
A python-based asynchronous mock execution script designed to be pointed at the `edge-ingress` bindings. It packages the raw hex structure matching `mock_ingress_payload.hex` natively establishing socket stability over standard loops evaluating performance capabilities.

### 3. `simulate_bank_host.py`
A persistent Python mock TCP server designed to receive mapped payloads streaming exclusively from the `edge-egress` instances. It decodes the exact 2-byte loop headers returning transaction mappings accurately isolating test results safely away from SEDA testing logic.

## Usage
Once the `main.rs` execution binary is configured tying the pipelines natively, these scripts will act as isolated mock elements reproducing the 10,000 TPS hardware constraints simulating raw network socket noise natively without needing authentic Bank integration hardware!
