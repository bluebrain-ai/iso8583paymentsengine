const express = require('express');
const http = require('http');
const path = require('path');

const app = express();

// Upstream targets
const SIMULATOR_TARGET = { host: '127.0.0.1', port: 3005 };
const ORCHESTRATOR_TARGET = { host: '127.0.0.1', port: 9001 };
const OPS_CONTROL_PLANE_TARGET = { host: '127.0.0.1', port: 3003 };

/**
 * Generic reverse-proxy factory.
 * @param {object} target - { host, port }
 * @param {string} [mountPath] - The Express mount path (e.g. '/sim'), so we can restore the full path
 * @param {object} [rewrite] - { from: RegExp, to: string } path rewrite applied to originalUrl
 */
function makeProxy(target, mountPath, rewrite) {
    return function (req, res) {
        // req.url is the path AFTER the mount point. Reconstruct the full path from originalUrl.
        let upstreamPath = req.originalUrl;
        if (rewrite) {
            upstreamPath = upstreamPath.replace(rewrite.from, rewrite.to);
        }

        const options = {
            hostname: target.host,
            port: target.port,
            path: upstreamPath,
            method: req.method,
            headers: { ...req.headers, host: `${target.host}:${target.port}` },
        };

        const proxyReq = http.request(options, (proxyRes) => {
            res.writeHead(proxyRes.statusCode, proxyRes.headers);
            proxyRes.pipe(res, { end: true });
        });

        proxyReq.on('error', (err) => {
            console.error(`[BFF Proxy Error] ${req.originalUrl} -> ${target.host}:${target.port}${upstreamPath}`, err.message);
            if (!res.headersSent) res.status(502).json({ error: 'Bad Gateway', detail: err.message });
        });

        req.pipe(proxyReq, { end: true });
    };
}

// ── Chaos Lab Orchestrator (port 9000) ──────────────────────────────────────
app.use('/sim/inject', makeProxy(ORCHESTRATOR_TARGET, '/sim/inject', { from: /^\/sim\/inject/, to: '/api/inject/raw' }));
app.use('/sim/chaos', makeProxy(ORCHESTRATOR_TARGET, '/sim/chaos', { from: /^\/sim\/chaos/, to: '/api/process' }));

// ── Rust Load Generator (port 3005) ─────────────────────────────────────────
app.use('/sim', makeProxy(SIMULATOR_TARGET));
app.use('/api/dashmap', makeProxy(SIMULATOR_TARGET));
app.use('/api/journal', makeProxy(SIMULATOR_TARGET));

// ── Rust Control Plane (port 3003) ─────────────────────────────────────────
app.use('/api/ledger', makeProxy(OPS_CONTROL_PLANE_TARGET));

// ── Static UI (ops-ui/index.html, no caching) ────────────────────────────────
app.use(express.static(path.join(__dirname, '../ops-ui'), {
    etag: false,
    lastModified: false,
    setHeaders: (res) => res.set('Cache-Control', 'no-store'),
}));

const PORT = 3001;
app.listen(PORT, '0.0.0.0', () => {
    console.log(`[OK] Ops BFF Edge Gateway Active on Port ${PORT}`);
    console.log(`     -> /sim/*        => localhost:${SIMULATOR_TARGET.port}`);
    console.log(`     -> /api/dashmap  => localhost:${SIMULATOR_TARGET.port}`);
    console.log(`     -> /api/journal  => localhost:${SIMULATOR_TARGET.port}`);
    console.log(`     -> /sim/inject   => localhost:${ORCHESTRATOR_TARGET.port}/api/inject/raw`);
    console.log(`     -> /sim/chaos    => localhost:${ORCHESTRATOR_TARGET.port}/api/process`);
});
