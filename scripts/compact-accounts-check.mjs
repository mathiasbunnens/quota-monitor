// Compact window (React) with the models emitted by --quota-ui-smoke: header « CODEX ↗ » per provider,
// account cards (name, plan, state; no chevron, no detail), 5 h / 7 j reset lines, + per provider. Offline: the real CompactPanel, recorded handlers.
//   node scripts/compact-accounts-check.mjs         (builds from this checkout; AFTER models: evidence/t_e0f6e24a/native)
//   BEFORE_ROOT=<dir with src/ and node_modules/> BEFORE_MODELS=<dir> to also capture the previous build.
import {build} from 'vite';
import react from '@vitejs/plugin-react';
import {spawn} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';
const out=path.resolve(process.env.SHOTS||'evidence/t_e0f6e24a/shots-compact');fs.mkdirSync(out,{recursive:true});
const variants={after:{root:process.cwd(),models:path.resolve(process.env.AFTER_MODELS||'evidence/t_e0f6e24a/native')}};
if(process.env.BEFORE_ROOT)variants.before={root:path.resolve(process.env.BEFORE_ROOT),models:path.resolve(process.env.BEFORE_MODELS)};
const sites={};
for(const [name,v] of Object.entries(variants)){
 const dir=path.join(v.root,'node_modules/.cache/qm-compact-'+name);fs.mkdirSync(dir,{recursive:true});
 fs.writeFileSync(path.join(dir,'fixture.tsx'),`import React from 'react';import{createRoot}from'react-dom/client';import{CompactPanel}from ${JSON.stringify(path.join(v.root,'src/features/compact/CompactPanel.tsx'))};import ExternalIcon from ${JSON.stringify(path.join(v.root,'src/features/ui/ExternalIcon.tsx'))};import ${JSON.stringify(path.join(v.root,'src/features/compact/compact.scss'))};window.__calls=[];const q=new URLSearchParams(location.search).get('model');fetch('./models/'+q+'.json').then(r=>r.json()).then(panel=>createRoot(document.getElementById('root')).render(<CompactPanel panel={panel} error={null} pending={null} onSettings={()=>{}} onUpdate={()=>{}} onConnect={()=>{}} onUsage={k=>window.__calls.push(['usage',k])} onAccount={(k,id)=>window.__calls.push(['account',k,id])} onAdd={k=>window.__calls.push(['add',k])} usageIcon={<ExternalIcon/>}/>));`);
 fs.writeFileSync(path.join(dir,'index.html'),'<!doctype html><html lang="fr"><meta charset="utf-8"><title>compact fixture</title><style>*{box-sizing:border-box}body{margin:0}</style><div id="root"></div><script type="module" src="./fixture.tsx"></script></html>');
 await build({configFile:false,root:dir,logLevel:'warn',plugins:[react()],build:{outDir:path.join(dir,'site'),emptyOutDir:true}});
 fs.mkdirSync(path.join(dir,'site/models'),{recursive:true});
 for(const f of fs.readdirSync(v.models).filter(f=>f.endsWith('.json')))fs.copyFileSync(path.join(v.models,f),path.join(dir,'site/models',f));
 sites[name]=path.join(dir,'site');
}
const ports={before:1595,after:1596};const types={js:'text/javascript',css:'text/css',html:'text/html',json:'application/json',svg:'image/svg+xml'};
const servers=Object.entries(sites).map(([k,root])=>http.createServer((req,res)=>{const u=new URL(req.url,'http://x').pathname;const f=path.join(root,u.endsWith('/')?u+'index.html':u);if(!f.startsWith(root)||!fs.existsSync(f)){res.writeHead(404);return res.end();}res.setHeader('Content-Type',types[f.split('.').pop()]||'application/octet-stream');res.end(fs.readFileSync(f));}).listen(ports[k],'127.0.0.1'));
const shell=process.env.CHROME_SHELL||process.env.HOME+'/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome=spawn(shell,['--remote-debugging-port=9394','--user-data-dir='+path.join(out,'.profile'),'--no-first-run','--hide-scrollbars','about:blank'],{stdio:'ignore'});
const kill=setTimeout(()=>{chrome.kill('SIGKILL');process.exit(2);},240000);
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let ws;const errors=[];const report={};
try{
 let target;for(let i=0;i<50&&!target;i++){await sleep(200);try{target=(await(await fetch('http://127.0.0.1:9394/json')).json()).find(t=>t.type==='page');}catch{}}
 ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let id=0;const pending=new Map();ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}else if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.text);});
 const cdp=(method,params={})=>new Promise((resolve,reject)=>{const n=++id;pending.set(n,m=>m.error?reject(Error(m.error.message)):resolve(m.result));ws.send(JSON.stringify({id:n,method,params}));});
 const ev=async e=>(await cdp('Runtime.evaluate',{expression:e,returnByValue:true,awaitPromise:true})).result.value;
 const shot=async name=>fs.writeFileSync(path.join(out,name+'.png'),Buffer.from((await cdp('Page.captureScreenshot',{format:'png',captureBeyondViewport:true})).data,'base64'));
 await cdp('Page.enable');await cdp('Runtime.enable');
 const open=async(which,model,scheme='light',width=324)=>{await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:scheme},{name:'prefers-reduced-motion',value:'reduce'}]});await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:2,mobile:false});await cdp('Page.navigate',{url:`http://127.0.0.1:${ports[which]}/?model=${model}`});await sleep(700);};
 const scenes=['single-account','single-account-classic','codex-and-claude-multi','codex-multi-accounts','codex-multi-accounts-classic','multi-accounts-classic','single-fable','multi-accounts','multi-fable','multi-fable-classic'];
 for(const scheme of ['light','dark'])for(const which of Object.keys(sites))for(const m of scenes){await open(which,m,scheme);await shot(`compact-${m}-${which}-${scheme}`);}

 // ---- assertions on the new build ----
 await open('after','codex-and-claude-multi');
 const model=await ev(`(()=>{const gs=[...document.querySelectorAll('.compact-group')];return gs.map(g=>({heading:g.querySelector('.compact-group__head').innerText.trim(),usage:g.querySelectorAll('.compact-group__usage').length,arrow:!!g.querySelector('.compact-group__usage .external-icon'),cards:[...g.querySelectorAll('.compact-provider')].map(c=>{const h=c.querySelector('.compact-provider__head');const kids=[...h.children].map(k=>k.className.split(' ')[0]);const rs=[...h.children].map(k=>k.getBoundingClientRect());return {name:c.querySelector('.compact-provider__name').textContent,kids,state:c.querySelector('.compact-provider__state')?.textContent,chevron:c.querySelector('.compact-provider__chevron')?.getAttribute('aria-label'),inOrder:rs.every((r,i)=>i===0||r.left>=rs[i-1].right-1),inside:rs.every(r=>r.right<=h.getBoundingClientRect().right+1)}}),plus:g.querySelector('.compact-group__add')?.getAttribute('aria-label'),plusLast:g.lastElementChild.className==='compact-group__footer'}))})()`);
 report.model=model;
 assert.deepEqual(model.map(g=>[g.heading,g.usage,g.arrow]),[['CODEX',1,true],['CLAUDE',1,true]]);
 for(const g of model){assert.ok(g.plusLast&&g.plus,JSON.stringify(g));for(const c of g.cards){assert.ok(c.inOrder&&c.inside,JSON.stringify(c));assert.ok(c.state,JSON.stringify(c));assert.deepEqual(c.kids.slice(0,2),['compact-provider__name','compact-provider__badge']);assert.equal(c.kids[c.kids.length-1],'compact-provider__state');}}
 assert.deepEqual(model.map(g=>g.cards.map(c=>c.name)),[['Principal','Travail'],['Personnel','Travail']]);
 assert.deepEqual(model.map(g=>g.plus),['Ajouter un compte Codex','Ajouter un compte Claude']);
 // routing: header ↗ and each +; no account chevron exists
 await ev(`(()=>{const q=s=>[...document.querySelectorAll(s)];q('.compact-group__usage').forEach(b=>b.click());q('.compact-group__add').forEach(b=>b.click());})()`);
 const calls=await ev(`window.__calls`);report.calls=calls;
 assert.deepEqual(calls.slice(0,2),[['usage','codex'],['usage','claude']]);
 assert.deepEqual(calls.slice(2),[['add','codex'],['add','claude']]);
 assert.equal(await ev(`document.querySelectorAll('.compact-provider__chevron,[aria-label^="Détail"]').length`),0,'no chevron, no detail');
 // Compacte: each pair has its reset line under the bars (5 h left, 7 j right), Fable is a third fine track
 const pairs=await ev(`[...document.querySelectorAll('.compact-pair')].map(p=>({resets:[...p.querySelectorAll('.compact-pair__resets span')].map(s=>s.textContent),height:Math.round(p.getBoundingClientRect().height),clipped:[...p.querySelectorAll('.compact-pair__resets span,.compact-pair__values span')].some(s=>s.scrollWidth>s.clientWidth+1)}))`);
 report.pairs=pairs;assert.ok(pairs.length>=3);for(const p of pairs){assert.equal(p.resets.length,2);assert.ok(p.resets.every(t=>/^Réinit\. /.test(t)),JSON.stringify(p));assert.equal(p.height,60);assert.equal(p.clipped,false,JSON.stringify(p));}
 // sizes, no overflow, both schemes and widths
 for(const scheme of ['light','dark'])for(const width of [300,324,420]){await open('after','codex-multi-accounts',scheme,width);
  const g=await ev(`({overflow:document.documentElement.scrollWidth>innerWidth,heads:[...document.querySelectorAll('.compact-provider__head')].every(h=>h.scrollWidth<=h.clientWidth+1),states:[...document.querySelectorAll('.compact-provider__state')].map(s=>s.textContent),chevrons:document.querySelectorAll('.compact-provider__chevron').length})`);
  assert.equal(g.chevrons,0);assert.equal(g.overflow,false,`${scheme} ${width}`);assert.ok(g.heads,`${scheme} ${width}: head clipped`);}
 await open('after','codex-multi-accounts');
 assert.ok((await ev(`[...document.querySelectorAll('.compact-provider__state')].map(s=>s.textContent)`)).includes('Non connecté'));
 // classic presentation keeps the same structure
 await open('after','codex-multi-accounts-classic');
 assert.deepEqual(await ev(`({groups:document.querySelectorAll('.compact-group').length,usage:document.querySelectorAll('.compact-group__usage').length,pairs:document.querySelectorAll('.compact-pair').length})`),{groups:1,usage:1,pairs:0});
 await open('after','single-account');
 assert.deepEqual(await ev(`({groups:document.querySelectorAll('.compact-group').length,plus:document.querySelectorAll('.compact-group__add').length,chevrons:document.querySelectorAll('.compact-provider__chevron').length})`),{groups:1,plus:1,chevrons:0});
 assert.deepEqual(errors,[]);
 fs.writeFileSync(path.join(out,'report.json'),JSON.stringify(report,null,2));console.log('COMPACT_ACCOUNTS_PASS',Object.keys(report).length);
}finally{clearTimeout(kill);ws?.close();chrome.kill();servers.forEach(s=>s.close());}
