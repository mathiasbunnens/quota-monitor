// Dashboard (React, Windows/Linux window): header « Codex ↗ » / « Claude ↗ » per provider, account cards
// (name + plan on one line, reset, state, chevron), + per provider. Offline: real React views, mocked IPC
// (scripts/claude-renderer-fixture.mjs), headless Chrome over CDP.
//   node scripts/claude-renderer-fixture.mjs node_modules/.cache/qm-b
//   AFTER_SITE=node_modules/.cache/qm-b/site [BEFORE_SITE=…] [SHOTS=dir] node scripts/dashboard-accounts-check.mjs
// Assertions run on AFTER_SITE; screenshots of BEFORE_SITE are taken when it is given.
import {spawn} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';
const out=path.resolve(process.env.SHOTS||'node_modules/.cache/dash-shots');fs.mkdirSync(out,{recursive:true});
const roots={after:path.resolve(process.env.AFTER_SITE||'node_modules/.cache/qm-b/site')};
if(process.env.BEFORE_SITE)roots.before=path.resolve(process.env.BEFORE_SITE);
const ports={before:1593,after:1594};
const types={js:'text/javascript',css:'text/css',html:'text/html',json:'application/json',svg:'image/svg+xml',png:'image/png'};
const servers=Object.entries(roots).map(([k,root])=>http.createServer((req,res)=>{const u=new URL(req.url,'http://x').pathname;const f=path.join(root,u.endsWith('/')?u+'index.html':u);if(!f.startsWith(root)||!fs.existsSync(f)){res.writeHead(404);return res.end();}res.setHeader('Content-Type',types[f.split('.').pop()]||'application/octet-stream');res.end(fs.readFileSync(f));}).listen(ports[k],'127.0.0.1'));
const shell=process.env.CHROME_SHELL||process.env.HOME+'/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome=spawn(shell,['--remote-debugging-port=9393','--user-data-dir='+path.join(out,'.profile'),'--no-first-run','--hide-scrollbars','about:blank'],{stdio:'ignore'});
const kill=setTimeout(()=>{chrome.kill('SIGKILL');process.exit(2);},270000);
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let ws;const errors=[];const report={};
try{
 let target;for(let i=0;i<50&&!target;i++){await sleep(200);try{target=(await(await fetch('http://127.0.0.1:9393/json')).json()).find(t=>t.type==='page');}catch{}}
 ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let id=0;const pending=new Map();ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}else if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.text);});
 const cdp=(method,params={})=>new Promise((resolve,reject)=>{const n=++id;pending.set(n,m=>m.error?reject(Error(m.error.message)):resolve(m.result));ws.send(JSON.stringify({id:n,method,params}));});
 const ev=async e=>(await cdp('Runtime.evaluate',{expression:e,returnByValue:true,awaitPromise:true})).result.value;
 const shot=async name=>fs.writeFileSync(path.join(out,name+'.png'),Buffer.from((await cdp('Page.captureScreenshot',{format:'png',captureBeyondViewport:true})).data,'base64'));
 await cdp('Page.enable');await cdp('Runtime.enable');
 const open=async(which,query,scheme='light',width=520)=>{await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:scheme},{name:'prefers-reduced-motion',value:'reduce'}]});await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:2,mobile:false});await cdp('Page.navigate',{url:`http://127.0.0.1:${ports[which]}/?${query}`});await sleep(800);};
 const calls=()=>ev(`window.__fixture.calls.filter(c=>!c.command.startsWith('get_')&&c.command!=='fit_settings_window').map(c=>[c.command,c.payload])`);
 const click=sel=>ev(`document.querySelector(${JSON.stringify(sel)}).click()`);
 const common='codex=yes&clock=controlled&platform=macos';
 const scenes={
  'one-compact':`${common}&scenario=one&codexAccounts=one&layout=compact`,
  'one-classic':`${common}&scenario=one&codexAccounts=one&layout=classic`,
  'multi-compact':`${common}&scenario=multi&codexAccounts=multi&layout=compact`,
  'multi-classic':`${common}&scenario=multi&codexAccounts=multi&layout=classic`,
  'signedout-compact':`${common}&scenario=signed-out&codexAccounts=multi&layout=compact`,
 };
 // ---- screenshots, before / after, both schemes ----
 for(const scheme of ['light','dark'])for(const which of Object.keys(roots))for(const [name,query] of Object.entries(scenes)){await open(which,query,scheme);await shot(`dashboard-${name}-${which}-${scheme}`);}

 // ---- assertions on the new build ----
 await open('after',scenes['multi-compact']);
 const model=await ev(`(()=>{const gs=[...document.querySelectorAll('.provider-group')];return gs.map(g=>{const h=g.querySelector('h2');const cs=[...g.querySelectorAll('.provider-card')];return {heading:h.innerText.trim(),usage:g.querySelectorAll('.provider-card__usage').length,cards:cs.map(c=>{const t=c.querySelector('.provider-card__title');const n=t.querySelector('.provider-card__name').getBoundingClientRect();const b=t.querySelector('.plan-badge')?.getBoundingClientRect();const pill=c.querySelector('.provider-card__pill').getBoundingClientRect();const chev=c.querySelector('.provider-card__chevron').getBoundingClientRect();const reset=c.querySelector('.reset-chip')?.getBoundingClientRect();return {name:t.querySelector('.provider-card__name').textContent,badge:b?t.querySelector('.plan-badge').textContent:null,sameLine:b?Math.abs((b.top+b.height/2)-(n.top+n.height/2))<4:null,pill:c.querySelector('.provider-card__pill').textContent,chevronAfterPill:chev.left>=pill.right-1,resetBeforePill:reset?reset.right<=pill.left+1:null,label:c.querySelector('.provider-card__chevron').getAttribute('aria-label')}}),plus:(()=>{const b=g.querySelector('.add-account__button');const r=b.getBoundingClientRect();const last=cs[cs.length-1].getBoundingClientRect();const gr=g.getBoundingClientRect();return {label:b.getAttribute('aria-label'),below:r.top>=last.bottom-1,right:Math.round(gr.right-r.right)}})()}})})()`);
 report.model=model;
 assert.deepEqual(model.map(g=>[g.heading,g.usage]),[['Codex',1],['Claude',1]]);
 assert.deepEqual(model[0].cards.map(c=>c.name),['Principal','Travail','Perso','Alias','detecte']);
 assert.deepEqual(model[1].cards.map(c=>c.name),['Personnel','Travail']);
 for(const g of model){for(const c of g.cards){assert.ok(c.chevronAfterPill,JSON.stringify(c));if(c.badge)assert.equal(c.sameLine,true,'badge beside the title: '+JSON.stringify(c));if(c.resetBeforePill!==null)assert.equal(c.resetBeforePill,true);}assert.ok(g.plus.below&&g.plus.right<=2,JSON.stringify(g.plus));}
 assert.deepEqual(model[0].cards.map(c=>c.pill),['Connecté','Connecté','Non connecté','Même compte','Connecté']);
 assert.deepEqual(model.map(g=>g.plus.label),['Ajouter un compte Codex','Ajouter un compte Claude']);
 assert.equal(await ev(`[...document.querySelectorAll('.dashboard button')].filter(b=>b.textContent.trim()==='Détails').length`),0,'no « Détails » button');
 // chevron → detail of THAT account only, then closes
 await click('[aria-label="Détail Codex Travail"]');await sleep(150);
 const detail=await ev(`(()=>{const ds=[...document.querySelectorAll('.provider-card__details')];return {count:ds.length,owner:ds[0]?.closest('.provider-card').getAttribute('aria-label'),paths:[...document.querySelectorAll('.provider-card__path')].map(p=>p.textContent),expanded:document.querySelector('[aria-label="Détail Codex Travail"]').getAttribute('aria-expanded')}})()`);
 report.detail=detail;assert.deepEqual(detail,{count:1,owner:'Quotas Codex · Travail',paths:['/fixtures/.codex-travail'],expanded:'true'});
 await shot('dashboard-multi-compact-detail-after-light');
 await click('[aria-label="Détail Claude Travail"]');await sleep(150);
 assert.deepEqual(await ev(`[...document.querySelectorAll('.provider-card__path')].map(p=>p.textContent)`),['/fixtures/.codex-travail','/fixtures/.claude-work']);
 await click('[aria-label="Détail Codex Travail"]');await sleep(100);
 assert.deepEqual(await ev(`[...document.querySelectorAll('.provider-card__path')].map(p=>p.textContent)`),['/fixtures/.claude-work']);
 // usage arrows: once per provider, opens that provider's page
 await click('#codex-group button');await click('#claude-group button');
 assert.deepEqual((await calls()).map(c=>c[0]),['open_usage_details','open_claude_usage']);
 // + routes to its own provider: Claude + → chooser → add_claude_profile; Codex + → add_codex_profile
 await open('after',scenes['multi-compact']+'&pick=/fixtures/.claude-work&pickCodex=/fixtures/.codex-travail');
 await click('[aria-label="Ajouter un compte Claude"]');await sleep(150);
 const flow=await ev(`(()=>{const f=document.querySelector('#claude-group').closest('section').querySelector('.add-account__flow');return {chooser:!!f.querySelector('.add-account__choose'),input:f.querySelector('input').getAttribute('aria-label'),dialogs:document.querySelectorAll('dialog,[role=dialog]').length,codexFlow:!!document.querySelector('#codex-group').closest('section').querySelector('.add-account__flow')}})()`);
 report.flow=flow;assert.deepEqual(flow,{chooser:true,input:'Chemin absolu d’un profil Claude Code',dialogs:0,codexFlow:false});
 await shot('dashboard-multi-compact-add-after-light');
 await ev(`document.querySelector('#claude-group').closest('section').querySelector('.add-account__choose').click()`);await sleep(500);
 await click('[aria-label="Ajouter un compte Codex"]');await sleep(150);
 await ev(`document.querySelector('#codex-group').closest('section').querySelector('.add-account__choose').click()`);await sleep(500);
 const routed=await calls();report.routed=routed;
 assert.deepEqual(routed.map(c=>c[0]),['choose_claude_profile_folder','add_claude_profile','choose_codex_profile_folder','add_codex_profile']);
 assert.deepEqual(routed[1][1],{path:'/fixtures/.claude-work'});assert.deepEqual(routed[3][1],{path:'/fixtures/.codex-travail'});
 // without a native chooser (Windows/Linux): the path field alone
 await open('after',scenes['multi-compact'].replace('platform=macos','platform=linux'));
 await click('[aria-label="Ajouter un compte Codex"]');await sleep(150);
 assert.deepEqual(await ev(`({chooser:document.querySelectorAll('.add-account__choose').length,input:document.querySelectorAll('.add-account__manual input').length})`),{chooser:0,input:1});
 // Compacte / Classique: same structure, paired bars only in Compacte
 for(const [name,expectBars] of [['multi-compact',true],['multi-classic',false],['one-compact',true],['one-classic',false]]){
  await open('after',scenes[name]);
  const s=await ev(`({groups:document.querySelectorAll('.provider-group').length,usage:document.querySelectorAll('.provider-card__usage').length,plus:document.querySelectorAll('.add-account__button').length,bars:document.querySelectorAll('.provider-card__paired-bars').length>0,overflow:document.documentElement.scrollWidth>window.innerWidth})`);
  assert.deepEqual(s,{groups:2,usage:2,plus:2,bars:expectBars,overflow:false},name);
 }
 // Classique with both providers side by side: the chevron also reveals the limits hidden by the short card, then the folder
 await open('after',scenes['one-classic']);
 await click('[aria-label="Détail Codex Principal"]');await sleep(150);
 const classic=await ev(`(()=>{const d=document.querySelector('.provider-card__details');return {rows:d.querySelectorAll('.provider-row').length,reset:/Réinitialisation/.test(d.innerText),path:d.querySelector('.provider-card__path').textContent,owner:d.closest('.provider-card').getAttribute('aria-label')}})()`);
 report.classic=classic;assert.deepEqual(classic,{rows:1,reset:true,path:'~/.codex',owner:'Quotas Codex · Principal'});
 await shot('dashboard-one-classic-detail-after-light');
 // a signed-out Claude account: state shown on its card
 await open('after',scenes['signedout-compact']);
 assert.equal(await ev(`[...document.querySelectorAll('#claude-group')][0].closest('section').querySelectorAll('.provider-card__pill')[1].textContent`),'Non connecté');
 await shot('dashboard-signedout-compact-after-light');
 // wide window, stacked groups keep one header each
 await open('after',scenes['multi-compact'],'light',900);await shot('dashboard-multi-compact-wide-after-light');
 assert.deepEqual(errors,[]);
 fs.writeFileSync(path.join(out,'report.json'),JSON.stringify(report,null,2));console.log('DASHBOARD_ACCOUNTS_PASS',Object.keys(report).length);
}finally{clearTimeout(kill);ws?.close();chrome.kill();servers.forEach(s=>s.close());}
