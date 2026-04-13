const express = require('express');
const path = require('path');

const app = express();

// Parse Windows Native Orchestrator port arguments "-p 3002"
let port = process.env.PORT || 3002;
const pIndex = process.argv.indexOf('-p');
if (pIndex > -1 && process.argv.length > pIndex + 1) {
  port = parseInt(process.argv[pIndex + 1], 10) || port;
}

app.get('*', (req, res) => {
  res.sendFile(path.join(__dirname, 'dashboard.html'));
});

app.listen(port, '0.0.0.0', () => {
  console.log(`[OPS-UI] Standalone Node Engine serving Ops Control Matrix natively on port ${port}...`);
});
