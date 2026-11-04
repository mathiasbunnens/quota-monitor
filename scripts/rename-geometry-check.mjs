// Réglages → Fournisseurs: renaming an account (pencil) must neither change the height of the
// row / content / window, nor stretch the field over the whole row. Geometry is measured in headless
// Chrome (real React views + styles, mocked IPC) before activation, with the field open, and while typing a very long name.
//   node scripts/claude-renderer-fixture.mjs node_modules/.cache/rn-after
//   SITE=node_modules/.cache/rn-after/site [SHOTS=dir] [MEASURE_ONLY=1] node scripts/rename-geometry-check.mjs
// MEASURE_ONLY=1 prints the numbers without asserting (used on the pre-fix build to show the RED state).
import {spawn} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';
const out=path.resolve(process.env.SHOTS||'node_modules/.cache/rename-shots');fs.mkdirSync(out,{recursive:true});
const site=path.resolve(process.env.SITE||'node_modules/.cache/rn-after/site');
const measureOnly=!!process.env.MEASURE_ONLY;const tag=process.env.TAG||'after';
const types={js:'text/javascript',css:'text/css',html:'text/html',json:'application/json',svg:'image/svg+xml',png:'image/png'};
const server=http.createServer((req,res)=>{const u=new URL(req.url,'http://x').pathname;const f=path.join(site,u.endsWith('/')?u+'index.html':u);if(!f.startsWith(site)||!fs.existsSync(f)){res.writeHead(404);return res.end();}res.setHeader('Content-Type',types[f.split('.').pop()]||'application/octet-stream');res.end(fs.readFileSync(f));}).listen(1597,'127.0.0.1');
const shell=process.env.CHROME_SHELL||process.env.HOME+'/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome=spawn(shell,['--remote-debugging-port=9397','--user-data-dir='+path.join(out,'.profile'),'--no-first-run','--hide-scrollbars','about:blank'],{stdio:'ignore'});
const kill=setTimeout(()=>{chrome.kill('SIGKILL');process.exit(2);},200000);
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let ws;const errors=[];const report={};
try{
 let target;for(let i=0;i<50&&!target;i++){await sleep(200);try{target=(await(await fetch('http://127.0.0.1:9397/json')).json()).find(t=>t.type==='page');}catch{}}
 ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let id=0;const pending=new Map();ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}else if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.text);});
 const cdp=(method,params={})=>new Promise((resolve,reject)=>{const n=++id;pending.set(n,m=>m.error?reject(Error(m.error.message)):resolve(m.result));ws.send(JSON.stringify({id:n,method,params}));});
 const ev=async e=>(await cdp('Runtime.evaluate',{expression:e,returnByValue:true,awaitPromise:true})).result.value;
 const shot=async name=>fs.writeFileSync(path.join(out,name+'.png'),Buffer.from((await cdp('Page.captureScreenshot',{format:'png'})).data,'base64'));
 await cdp('Page.enable');await cdp('Runtime.enable');
 const open=async(query,scheme,width)=>{await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:scheme},{name:'prefers-reduced-motion',value:'reduce'}]});await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:2,mobile:false});await cdp('Page.navigate',{url:`http://127.0.0.1:1597/?${query}`});await sleep(700);await ev(`document.getElementById('tab-providers').click()`);await sleep(300);};
 // Everything the window height depends on: the fitted content (what `fit_settings_window` sends) and the row.
 const measure=(provider,name)=>ev(`(()=>{const sec=document.getElementById('${provider}-heading').closest('section');const rows=[...sec.querySelectorAll('.claude-accounts__row')];const li=rows.find(r=>r.querySelector('input.claude-accounts__rename'))||rows.find(r=>new RegExp(${JSON.stringify(name)}).test(r.innerText));const who=li.querySelector('.claude-accounts__who');const field=li.querySelector('input.claude-accounts__rename');const label=li.querySelector('.claude-accounts__label');const pill=li.querySelector('.prefs-pill').getBoundingClientRect();const sw=li.querySelector('.prefs-switch,.claude-accounts__pick');const lr=li.getBoundingClientRect();const f=field?.getBoundingClientRect();const l=label?.getBoundingClientRect();const root=document.querySelector('main.prefs').getBoundingClientRect();return {content:Math.round(root.height*100)/100,doc:document.documentElement.scrollHeight,row:Math.round(lr.height*100)/100,who:Math.round(who.getBoundingClientRect().height*100)/100,rowWidth:Math.round(lr.width),field:f?{w:Math.round(f.width),h:Math.round(f.height*100)/100,right:Math.round(f.right),left:Math.round(f.left)}:null,labelH:l?Math.round(l.height*100)/100:null,pillLeft:Math.round(pill.left),pillTop:Math.round(pill.top-lr.top),switchVisible:sw?sw.getBoundingClientRect().right<=lr.right+1:null,overflowX:document.documentElement.scrollWidth>window.innerWidth,rowsOnOneLine:Math.abs(pill.top+pill.height/2-(who.getBoundingClientRect().top+who.getBoundingClientRect().height/2))<14}})()`);
 const click=sel=>ev(`document.querySelector(${JSON.stringify(sel)}).click()`);
 const typeLong=async text=>{await cdp('Input.insertText',{text});await sleep(120);};
 const cases=[
  ['claude-single-primary','view=settings&scenario=one&clock=controlled&platform=macos','claude','Personnel','Renommer Personnel'],
  ['claude-multi-primary','view=settings&scenario=multi&clock=controlled&platform=macos','claude','Personnel','Renommer Personnel'],
  ['claude-multi-second','view=settings&scenario=multi&clock=controlled&platform=macos','claude','Travail','Renommer Travail'],
  ['codex-multi-primary','view=settings&scenario=multi&codex=yes&codexAccounts=multi&clock=controlled&platform=macos','codex','Principal','Renommer Principal'],
 ];
 const LONG='Compte professionnel de Mathias Bunnens — équipe plateforme et intégrations (principal)';
 for(const scheme of ['light','dark'])for(const width of [480,360])for(const [name,query,provider,who,label] of cases){
  await open(query,scheme,width);
  const before=await measure(provider,who);
  await click(`[aria-label="${label}"]`);await sleep(250);
  const editing=await measure(provider,'');
  const focused=await ev(`document.activeElement?.classList.contains('claude-accounts__rename')`);
  const named=await ev(`document.activeElement?.getAttribute('aria-label')`);
  await cdp('Input.dispatchKeyEvent',{type:'keyDown',key:'a',code:'KeyA',modifiers:4,windowsVirtualKeyCode:65});await cdp('Input.dispatchKeyEvent',{type:'keyUp',key:'a',code:'KeyA',modifiers:4,windowsVirtualKeyCode:65});
  await typeLong(LONG);await sleep(250);
  const typing=await measure(provider,'');
  if(scheme==='light'||width===360)await shot(`rename-${tag}-${name}-${width}-${scheme}`);
  const r={before,editing,typing,focused,named};report[`${name}-${width}-${scheme}`]=r;
  if(measureOnly)continue;
  const tag2=`${name}@${width}/${scheme}`;
  // same height everywhere: row, fitted content, document → the window never resizes
  for(const [k,state] of [['editing',editing],['typing',typing]]){
   assert.equal(state.row,before.row,`${tag2} row height ${k}: ${before.row} → ${state.row}`);
   assert.equal(state.content,before.content,`${tag2} content (window) height ${k}: ${before.content} → ${state.content}`);
   assert.equal(state.doc,before.doc,`${tag2} document height ${k}`);
   assert.equal(state.who,before.who,`${tag2} name block height ${k}`);
   // bounded width: never the whole line, the state and the visibility control stay on the row
   assert.ok(state.field,`${tag2} field present`);
   assert.ok(state.field.w<=260,`${tag2} field ${state.field.w}px > 260`);
   assert.ok(state.field.w<state.rowWidth*0.6||width===360&&state.field.w<=state.rowWidth*0.65,`${tag2} field ${state.field.w}px of a ${state.rowWidth}px row`);
   assert.ok(state.field.right<=state.pillLeft-4,`${tag2} field overlaps the state pill`);
   assert.equal(state.rowsOnOneLine,true,`${tag2} state/name on one line`);
   assert.notEqual(state.switchVisible,false,`${tag2} visibility control inside the row`);
   assert.equal(state.overflowX,false,`${tag2} horizontal overflow`);
   assert.ok(Math.abs(state.field.h-(before.labelH??state.field.h))<=8,`${tag2} field height ${state.field.h} vs label ${before.labelH}`);
  }
  assert.equal(typing.field.w,editing.field.w,`${tag2} field keeps its width while typing`);
  assert.equal(focused,true,`${tag2} field focused`);assert.match(named,/^Nom du compte /,`${tag2} accessible name`);
  // Escape cancels (name unchanged, focus back on the pencil); Enter on an unchanged draft closes too
  await cdp('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await cdp('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await sleep(250);
  const after=await measure(provider,who);
  assert.equal(after.content,before.content,`${tag2} back to the same height`);
  assert.equal(await ev(`!!document.querySelector('input.claude-accounts__rename')`),false,`${tag2} Escape closes`);
  assert.equal(await ev(`document.activeElement?.getAttribute('aria-label')`),label,`${tag2} focus back on the pencil`);
 }
 fs.writeFileSync(path.join(out,`rename-${tag}.json`),JSON.stringify(report,null,2));
 if(measureOnly){const worst=Object.entries(report).map(([k,r])=>[k,r.before.content,r.editing.content,r.typing.content,r.editing.field?.w,r.editing.row-r.before.row]);console.log(JSON.stringify(worst));}
 else console.log('RENAME_GEOMETRY_PASS',Object.keys(report).length);
 assert.deepEqual(errors,[]);
}finally{clearTimeout(kill);ws?.close();chrome.kill();server.close();}
