#!/bin/bash
echo "Starting Rust Test Orchestrator (Port 9000)..."
cd d:/cob2java/GitHub/ISOPaymentEngine/test-orchestrator
cargo run &

echo "Starting Node.js BFF Gateway (Port 3000)..."
cd d:/cob2java/GitHub/ISOPaymentEngine/ops-bff
npm start &

echo "Starting Next.js Unified Shell (Port 4000)..."
cd d:/cob2java/GitHub/ISOPaymentEngine/ops-ui
npm run dev &

echo "God Mode Command Center components are starting recursively in background."
wait
