// Settings → Fournisseurs (Codex AND Claude): account rows WITHOUT chevron or detail panel, automatic detection, + button.
// Offline: real React views, mocked IPC (scripts/claude-renderer-fixture.mjs), headless Chrome over CDP.
//   node scripts/claude-renderer-fixture.mjs node_modules/.cache/dac-after
//   AFTER_SITE=node_modules/.cache/dac-after/site [BEFORE_SITE=…] [SHOTS=dir] node scripts/accounts-detail-check.mjs
// Assertions always run on AFTER_SITE; screenshots of BEFORE_SITE are taken when it is given.
import {spawn} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';
const out=path.resolve(process.env.SHOTS||'node_modules/.cache/dac-shots');fs.mkdirSync(out,{recursive:true});
const roots={after:path.resolve(process.env.AFTER_SITE||'node_modules/.cache/dac-after/site')};
if(process.env.BEFORE_SITE)roots.before=path.resolve(process.env.BEFORE_SITE);
const ports={before:1591,after:1592};
const types={js:'text/javascript',css:'text/css',html:'text/html',json:'application/json',svg:'image/svg+xml',png:'image/png'};
const servers=Object.entries(roots).map(([k,root])=>http.createServer((req,res)=>{const u=new URL(req.url,'http://x').pathname;const f=path.join(root,u.endsWith('/')?u+'index.html':u);if(!f.startsWith(root)||!fs.existsSync(f)){res.writeHead(404);return res.end();}res.setHeader('Content-Type',types[f.split('.').pop()]||'application/octet-stream');res.end(fs.readFileSync(f));}).listen(ports[k],'127.0.0.1'));
const shell=process.env.CHROME_SHELL||process.env.HOME+'/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome=spawn(shell,['--remote-debugging-port=9392','--user-data-dir='+path.join(out,'.profile'),'--no-first-run','--hide-scrollbars','about:blank'],{stdio:'ignore'});
const kill=setTimeout(()=>{chrome.kill('SIGKILL');process.exit(2);},240000);
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let ws;const errors=[];const report={};
try{
 let target;for(let i=0;i<50&&!target;i++){await sleep(200);try{target=(await(await fetch('http://127.0.0.1:9392/json')).json()).find(t=>t.type==='page');}catch{}}
 ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let id=0;const pending=new Map();ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}else if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.text);});
 const cdp=(method,params={})=>new Promise((resolve,reject)=>{const n=++id;pending.set(n,m=>m.error?reject(Error(m.error.message)):resolve(m.result));ws.send(JSON.stringify({id:n,method,params}));});
 const ev=async e=>(await cdp('Runtime.evaluate',{expression:e,returnByValue:true,awaitPromise:true})).result.value;
 const shot=async name=>fs.writeFileSync(path.join(out,name+'.png'),Buffer.from((await cdp('Page.captureScreenshot',{format:'png',captureBeyondViewport:true})).data,'base64'));
 const key=async(k,code,vk,text)=>{await cdp('Input.dispatchKeyEvent',{type:'keyDown',key:k,code,text,windowsVirtualKeyCode:vk});await cdp('Input.dispatchKeyEvent',{type:'keyUp',key:k,code,windowsVirtualKeyCode:vk});await sleep(150);};
 await cdp('Page.enable');await cdp('Runtime.enable');
 const open=async(which,query,scheme='light',width=480)=>{await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:scheme},{name:'prefers-reduced-motion',value:'reduce'}]});await cdp('Emulation.setDeviceMetricsOverride',{width,height:1000,deviceScaleFactor:2,mobile:false});await cdp('Page.navigate',{url:`http://127.0.0.1:${ports[which]}/?${query}`});await sleep(700);await ev(`document.getElementById('tab-providers').click()`);await sleep(300);};
 const calls=()=>ev(`window.__fixture.calls.filter(c=>!c.command.startsWith('get_')&&!c.command.startsWith('choose_')&&c.command!=='fit_settings_window'&&c.command!=='take_settings_focus').map(c=>[c.command,c.payload])`);
 // helpers evaluated in the page: one provider section
 const sec=p=>`document.getElementById('${p}-heading').closest('section')`;
 const rows=p=>ev(`[...${sec(p)}.querySelectorAll('.claude-accounts__row')].map(r=>r.innerText.replace(/\\s+/g,' ').trim())`);
 const click=(sel)=>ev(`document.querySelector(${JSON.stringify(sel)}).click()`);
 // the + opens the single add flow; the chooser inside it adds the picked folder
 const addVia=async p=>{await click(`[aria-label="Ajouter un compte ${p}"]`);await sleep(150);await ev(`document.getElementById('${p.toLowerCase()}-heading').closest('section').querySelector('.add-account__choose').click()`);await sleep(500);};
 const base='view=settings&scenario=multi&codex=yes&codexAccounts=multi&clock=controlled&platform=macos&layout=compact';
 const one='view=settings&scenario=one&codex=yes&codexAccounts=one&clock=controlled&platform=macos&layout=compact';

 // ---- screenshots, before / after, both schemes ----
 for(const scheme of ['light','dark'])for(const which of Object.keys(roots))for(const [name,query] of [['one',one],['multi',base]]){
  await open(which,query,scheme);report[`${name}-${which}-${scheme}`]={rows:{codex:await rows('codex'),claude:await rows('claude')}};await shot(`settings-${name}-${which}-${scheme}`);
 }

 // ---- assertions on the new build ----
 await open('after',base);
 // 1. detected homes are plain accounts: states per account, nothing to unfold
 const codexRows=await rows('codex');report.codexRows=codexRows;
 assert.equal(codexRows.length,5);assert.ok(codexRows.every(r=>!/Détecté|Suivre|Ne plus suivre|Supprimer/.test(r)),codexRows.join('|'));
 assert.match(codexRows[2],/Perso.*Non connecté/);assert.match(codexRows[3],/Alias.*Même compte/);assert.match(codexRows[4],/detecte.*Connecté/);
 // 2. no chevron, no detail, no folder, no Supprimer, no connection block, on any row of any provider/scene
 for(const [name,query] of [['multi',base],['one',one],['signedout',base+'&codexPhase=signed_out']]){
  await open('after',query);
  const none=await ev(`({chevrons:document.querySelectorAll('[aria-label^="Détail"],[aria-controls*="detail"]').length,details:document.querySelectorAll('.claude-accounts__detail,.claude-accounts__path,.claude-accounts__remove').length,remove:/Supprimer/.test(document.getElementById('panel-providers').innerText),embedded:document.querySelectorAll('.codex-connection--embedded').length,rowParts:[...document.querySelectorAll('.claude-accounts__row')].every(r=>r.querySelector('.claude-accounts__who')&&r.querySelector('.prefs-pill')&&r.querySelector('[aria-label^="Renommer"]'))})`);
  report['none-'+name]=none;assert.deepEqual(none,{chevrons:0,details:0,remove:false,embedded:0,rowParts:true},name);
 }
 // the not-connected Codex keeps its full connection panel above the list (login stays reachable)
 await open('after',base+'&codexPhase=signed_out');
 assert.equal(await ev(`document.querySelectorAll('#codex-title').length`),1);
 // 3. + : one icon button per provider, bottom right, accessible names, no "Ajouter un compte" line
 await open('after',base);
 const plus=await ev(`(()=>{const f=p=>{const s=document.getElementById(p+'-heading').closest('section');const b=s.querySelector('.claude-accounts__add');const l=s.querySelector('.claude-accounts__list').getBoundingClientRect();const r=b.getBoundingClientRect();return {label:b.getAttribute('aria-label'),text:b.textContent.trim(),right:Math.round(l.right-r.right),below:r.top>=l.bottom-1,w:Math.round(r.width),h:Math.round(r.height),svg:!!b.querySelector('svg')}};return {codex:f('codex'),claude:f('claude')}})()`);
 report.plus=plus;
 for(const [p,label] of [['codex','Ajouter un compte Codex'],['claude','Ajouter un compte Claude']]){const v=plus[p];assert.equal(v.label,label);assert.equal(v.text,'');assert.ok(v.svg&&v.below&&v.right<=2&&v.w>=22&&v.h>=22,JSON.stringify(v));}
 // 4. + routes to the right provider (folder chooser then add_*_profile); no remove command exists in the UI any more
 await open('after',base+'&pickCodex=/fixtures/.codex-nouveau&pick=/fixtures/.claude-work');
 await addVia('Codex');
 let c=await calls();assert.deepEqual(c,[['add_codex_profile',{path:'/fixtures/.codex-nouveau'}]]);
 assert.equal((await rows('codex')).length,6);
 assert.ok(!(await ev(`window.__fixture.calls.some(c=>/untrack|delete|remove|login|logout/.test(c.command))`)),'no removal/login command');
 // 5. without a native folder chooser (not macOS) the + opens the absolute-path field
 await open('after',base.replace('platform=macos','platform=linux'));
 await click('[aria-label="Ajouter un compte Codex"]');await sleep(300);
 assert.deepEqual(await ev(`({focus:document.activeElement.getAttribute('aria-label'),open:!!document.querySelector('.add-account__manual')})`),{focus:'Chemin absolu d’un dossier CODEX_HOME',open:true});
 // 6. one flow, Escape closes it, a relative path is refused with no command
 await open('after',base);
 await click('[aria-label="Ajouter un compte Codex"]');await sleep(200);
 const flow=await ev(`(()=>{const f=document.querySelector('.add-account__flow');const s=document.getElementById('codex-heading').closest('section');return {choose:!!f.querySelector('.add-account__choose'),input:f.querySelector('input').getAttribute('aria-label'),inSection:s.contains(f),dialogs:document.querySelectorAll('dialog,[role=dialog]').length}})()`);
 report.flow=flow;assert.deepEqual(flow,{choose:true,input:'Chemin absolu d’un dossier CODEX_HOME',inSection:true,dialogs:0});
 await shot('settings-add-flow-after-light');
 await ev(`document.querySelector('.add-account__flow input').focus()`);await key('Escape','Escape',27);await sleep(100);
 assert.equal(await ev(`document.querySelectorAll('.add-account__flow').length`),0);
 await click('[aria-label="Ajouter un compte Claude"]');await sleep(100);
 await ev(`(()=>{const i=document.querySelector('.add-account__flow input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(i,'relatif/dossier');i.dispatchEvent(new Event('input',{bubbles:true}))})()`);await sleep(80);
 await ev(`document.querySelector('.add-account__flow form button[type=submit]').click()`);await sleep(150);
 assert.match(await ev(`document.querySelector('.add-account__flow [role=alert]')?.textContent||''`),/Chemin absolu requis/);
 assert.ok(!(await calls()).some(c=>c[0]==='add_claude_profile'));
 // 7. one « Principal » on the primary row; rename/visibility unchanged
 await open('after',base);
 const principals=await ev(`[...document.getElementById('codex-heading').closest('section').querySelectorAll('.claude-accounts__row')].map(r=>(r.querySelector('.claude-accounts__who').innerText.match(/Principal/g)||[]).length)`);
 report.principals=principals;assert.equal(principals[0],1,'one « Principal » on the primary row');

 // 8. pointed at from the compact window: Réglages lands on that provider's add flow; an account id opens nothing
 const landing=async q=>{await open('after',base+q);return ev(`({open:document.querySelectorAll('.claude-accounts__detail').length,flows:[...document.querySelectorAll('.add-account__flow')].map(f=>f.closest('section').getAttribute('aria-labelledby')),tab:document.querySelector('[role=tab][aria-selected=true]').id,taken:window.__fixture.calls.filter(c=>c.command==='take_settings_focus').length>0})`);};
 report.landing={claudeAdd:await landing('&focus=claude&focusAdd=1'),codexAdd:await landing('&focus=codex&focusAdd=1'),codexAccount:await landing('&focus=codex|profile:travail'),none:await landing('')};
 assert.deepEqual(report.landing.claudeAdd.flows,['claude-heading']);assert.deepEqual(report.landing.codexAdd.flows,['codex-heading']);
 assert.deepEqual(report.landing.codexAccount,{open:0,flows:[],tab:'tab-providers',taken:true});
 assert.deepEqual(report.landing.none,{open:0,flows:[],tab:'tab-providers',taken:true});
 assert.deepEqual(errors,[]);
 fs.writeFileSync(path.join(out,'report.json'),JSON.stringify(report,null,2));console.log('ACCOUNTS_DETAIL_PASS',Object.keys(report).length);
}finally{clearTimeout(kill);ws?.close();chrome.kill();servers.forEach(s=>s.close());}
