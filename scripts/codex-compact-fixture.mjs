// Actual compact React view, fed by JSON emitted by --quota-ui-smoke.
// No IPC, app identity, provider/auth or external actions.
import {build} from 'vite';
import react from '@vitejs/plugin-react';
import fs from 'node:fs';
import path from 'node:path';
const out=path.resolve('node_modules/.cache/codex-compact');
fs.mkdirSync(out,{recursive:true});
fs.writeFileSync(path.join(out,'fixture.tsx'),`import React from 'react';import{createRoot}from'react-dom/client';import{CompactPanel}from ${JSON.stringify(path.resolve('src/features/compact/CompactPanel.tsx'))};import ${JSON.stringify(path.resolve('src/features/compact/compact.scss'))};const name=new URLSearchParams(location.search).get('model')||'codex-compact';fetch('./models/'+name+'.json').then(r=>r.json()).then(panel=>createRoot(document.getElementById('root')).render(<CompactPanel panel={panel} error={null} pending={null} onSettings={()=>{}} onUpdate={()=>{}} onConnect={()=>{}}/>));`);
fs.writeFileSync(path.join(out,'index.html'),'<!doctype html><html lang="fr"><meta charset="utf-8"><title>Offline Codex compact fixture</title><style>*{box-sizing:border-box}body{margin:0}</style><div id="root"></div><script type="module" src="./fixture.tsx"></script></html>');
await build({configFile:false,root:out,plugins:[react()],build:{outDir:path.join(out,'site'),emptyOutDir:true}});
const models=path.resolve('evidence/t_33379e53/native');
fs.mkdirSync(path.join(out,'site/models'),{recursive:true});
for(const file of fs.readdirSync(models).filter(f=>f.endsWith('.json')))fs.copyFileSync(path.join(models,file),path.join(out,'site/models',file));
console.log('CODEX_COMPACT_FIXTURE_BUILT');
