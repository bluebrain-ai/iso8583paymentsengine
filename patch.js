const fs = require('fs');
let html = fs.readFileSync('ops-ui/index.html', 'utf8');

const chaosLabComponent = `
function ChaosLabView() {
  const [hex, setHex] = useState('');
  const [logs, setLogs] = useState([]);
  const [dialect, setDialect] = useState('Base24');
  const [mti, setMti] = useState('0200');
  const [bitmap, setBitmap] = useState('B220000000000000');
  const [field2, setField2] = useState('4111111111111111');
  const [field3, setField3] = useState('000000');
  const [field4, setField4] = useState('000000001000');
  
  // Terminal log ref
  const logEndRef = useRef(null);

  // Sync to Hex
  useEffect(() => {
    // A primitive mock of ISO hex generation based on fields (for UI demonstration)
    const mockLen = (mti.length + bitmap.length + field2.length + field3.length + field4.length).toString(16).padStart(4, '0');
    // For realistic testing, SDET manually mutates the hex. This just creates a baseline when inputs change
    setHex(mti + bitmap + Array.from(field2).map(c=>c.charCodeAt(0).toString(16)).join('') + field3 + field4);
  }, [mti, bitmap, field2, field3, field4]);

  useEffect(() => {
    // Tail websockets from the Rust Orchestrator via BFF on 3001
    // Actually the proxy is for HTTP, for websockets it's best to connect directly or through ws proxy
    // Connecting directly to orchestrator 9000 as per common test-lab topology
    const ws = new WebSocket('ws://127.0.0.1:9000/stream/logs');
    ws.onmessage = (e) => {
      setLogs(prev => [...prev.slice(-99), e.data]);
    };
    return () => ws.close();
  }, []);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [logs]);

  const injectHex = async () => {
    try {
      await fetch('/sim/inject', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ hex })
      });
      setLogs(prev => [...prev, '[SYSTEM] Hex injected successfully.']);
    } catch (e) {
      setLogs(prev => [...prev, '[ERROR] Hex injection failed.']);
    }
  };

  const killTarget = async (target) => {
    try {
      await fetch('/sim/chaos/' + target + '/kill', { method: 'POST' });
    } catch(e) {}
  };

  return (
    <div className="row" style={{ flex: 1, minHeight: 0 }}>
      {/* Target & Build Card */}
      <div className="card" style={{ flex: '1 1 50%', display: 'flex', flexDirection: 'column' }}>
        <div className="card-header">
           <span className="card-title">
             <Icon name="Wrench" className="w-4 h-4 text-amber-500" />
             Message Builder & Hex Injector
           </span>
        </div>
        <div className="card-body" style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 12 }}>
           <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
             <div>
               <div className="field-label">Dialect</div>
               <select value={dialect} onChange={e => setDialect(e.target.value)} style={{ width: '100%', background: 'var(--surface2)', border: '1px solid var(--border2)', color: 'white', padding: '6px' }}>
                 <option>Base24</option><option>Connex</option>
               </select>
             </div>
             <div>
               <div className="field-label">MTI</div>
               <input type="text" value={mti} onChange={e => setMti(e.target.value)} className="mono" />
             </div>
           </div>
           
           <div>
             <div className="field-label">Primary Bitmap (Hex)</div>
             <input type="text" value={bitmap} onChange={e => setBitmap(e.target.value)} className="mono" />
           </div>

           <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 12 }}>
             <div>
               <div className="field-label">Field 2 (PAN)</div>
               <input type="text" value={field2} onChange={e => setField2(e.target.value)} className="mono" />
             </div>
             <div>
               <div className="field-label">Field 3 (Proc)</div>
               <input type="text" value={field3} onChange={e => setField3(e.target.value)} className="mono" />
             </div>
             <div>
               <div className="field-label">Field 4 (Amt)</div>
               <input type="text" value={field4} onChange={e => setField4(e.target.value)} className="mono" />
             </div>
           </div>

           <div style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
             <div className="field-label">Raw OSI OSI Layer 7 Hex</div>
             <textarea className="mono code-block" style={{ flex: 1, width: '100%', resize: 'none', background: 'black', color: 'var(--emerald)' }} value={hex} onChange={e => setHex(e.target.value)} />
           </div>

           <button className="btn btn-danger w-full" onClick={injectHex}>Fire Raw TCP Injection</button>
        </div>
      </div>

      {/* Interactive Topology & Logs */}
      <div style={{ flex: '1 1 50%', display: 'flex', flexDirection: 'column', gap: 8 }}>
         <div className="card" style={{ flexShrink: 0 }}>
            <div className="card-header">
              <span className="card-title">
                 <Icon name="Crosshair" className="w-4 h-4 text-red-500" />
                 OS Chaos Targets
              </span>
              <span style={{ fontSize: 9, color: 'var(--red)' }}>Click node to KILL</span>
            </div>
            <div className="card-body" style={{ padding: 14 }}>
               <div style={{ display: 'flex', justifyContent: 'space-around', alignItems: 'center', height: 80 }}>
                 {['mock-bank-node', 'payment-daemon', 'mock_hsm'].map(target => (
                    <div key={target} onClick={() => killTarget(target)} style={{ width: 80, height: 80, borderRadius: '50%', background: '#1a0505', border: '2px dashed var(--red)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'crosshair', flexDirection: 'column', transition: 'all 0.2s' }} onMouseEnter={e => e.currentTarget.style.background = '#3f0b0b'} onMouseLeave={e => e.currentTarget.style.background = '#1a0505'}>
                       <Icon name="PowerOff" className="w-6 h-6 text-red-500 mb-1" />
                       <span style={{ fontSize: 9, color: 'var(--muted)', textAlign: 'center' }}>{target}</span>
                    </div>
                 ))}
               </div>
            </div>
         </div>

         <div className="card" style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div className="card-header">
               <span className="card-title">
                 <Icon name="Terminal" className="w-4 h-4 text-cyan-500" />
                 Trace Terminal
               </span>
               <span className="dot-pulse" style={{ color: 'var(--emerald)' }} />
            </div>
            <div className="card-body mono" style={{ background: '#000', padding: 10, fontSize: 10, overflowY: 'auto' }}>
               {logs.map((log, i) => {
                 let color = 'var(--muted)';
                 if (log.includes('[ERR]')) color = 'var(--red)';
                 if (log.includes('[OUT]')) color = 'var(--text)';
                 return <div key={i} style={{ color, marginBottom: 4 }}>{log}</div>;
               })}
               <div ref={logEndRef} />
            </div>
         </div>
      </div>
    </div>
  );
}

`;

html = html.replace('function App() {', chaosLabComponent + 'function App() {');

const appOldTabButtons = `<button onClick={() => setActiveTab('obs')} style={{ background: 'none', border: 'none', color: activeTab === 'obs' ? 'var(--cyan)' : 'var(--muted)', fontWeight: activeTab === 'obs' ? 700 : 500, cursor: 'pointer', fontSize: 12, borderBottom: activeTab === 'obs' ? '2px solid var(--cyan)' : '2px solid transparent', paddingBottom: 2 }}>Observability Hub</button>
            <button onClick={() => setActiveTab('cp')} style={{ background: 'none', border: 'none', color: activeTab === 'cp' ? 'var(--cyan)' : 'var(--muted)', fontWeight: activeTab === 'cp' ? 700 : 500, cursor: 'pointer', fontSize: 12, borderBottom: activeTab === 'cp' ? '2px solid var(--cyan)' : '2px solid transparent', paddingBottom: 2 }}>Operations Matrix</button>`;

const appNewTabButtons = `<button onClick={() => setActiveTab('obs')} style={{ background: 'none', border: 'none', color: activeTab === 'obs' ? 'var(--cyan)' : 'var(--muted)', fontWeight: activeTab === 'obs' ? 700 : 500, cursor: 'pointer', fontSize: 12, borderBottom: activeTab === 'obs' ? '2px solid var(--cyan)' : '2px solid transparent', paddingBottom: 2 }}>Observability Hub</button>
            <button onClick={() => setActiveTab('cp')} style={{ background: 'none', border: 'none', color: activeTab === 'cp' ? 'var(--cyan)' : 'var(--muted)', fontWeight: activeTab === 'cp' ? 700 : 500, cursor: 'pointer', fontSize: 12, borderBottom: activeTab === 'cp' ? '2px solid var(--cyan)' : '2px solid transparent', paddingBottom: 2 }}>Operations Matrix</button>
            <button onClick={() => setActiveTab('chaos')} style={{ background: 'none', border: 'none', color: activeTab === 'chaos' ? 'var(--cyan)' : 'var(--muted)', fontWeight: activeTab === 'chaos' ? 700 : 500, cursor: 'pointer', fontSize: 12, borderBottom: activeTab === 'chaos' ? '2px solid var(--cyan)' : '2px solid transparent', paddingBottom: 2 }}>Chaos &amp; Certification Lab</button>`;

html = html.replace(appOldTabButtons, appNewTabButtons);

html = html.replace("{activeTab === 'obs' ? <ObservabilityView /> : <ControlPlaneView />}", 
    "{activeTab === 'obs' ? <ObservabilityView /> : activeTab === 'cp' ? <ControlPlaneView /> : <ChaosLabView />}");

fs.writeFileSync('ops-ui/index.html', html);
console.log('Patched ops-ui/index.html');
