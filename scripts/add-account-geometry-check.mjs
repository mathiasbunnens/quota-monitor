// Offline + panel geometry regression; SITE, SHOTS, TAG, MEASURE_ONLY as rename-geometry-check.mjs.
import {spawn} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';
const out=path.resolve(process.env.SHOTS||'node_modules/.cache/add-shots');fs.mkdirSync(out,{recursive:true});
const site=path.resolve(process.env.SITE||'node_modules/.cache/add-before/site');
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
 const click=sel=>ev(`document.querySelector(${JSON.stringify(sel)}).click()`);
 const measure=()=>ev(`(()=>{const f=document.querySelector('.add-account__flow');const form=f?.querySelector('form');const input=f?.querySelector('input');const b=f?.querySelector('.add-account__choose');const root=document.querySelector('main.prefs,main.dashboard')||document.querySelector('#root');const r=x=>{if(!x)return null;const a=x.getBoundingClientRect();return {x:a.x,y:a.y,w:a.width,h:a.height,right:a.right,bottom:a.bottom}};return {content:r(root),window:innerHeight,document:document.documentElement.scrollHeight,flow:r(f),row:r(form),input:r(input),folder:r(b),margin:input?getComputedStyle(input).margin:null,overflow:document.documentElement.scrollWidth>innerWidth}})()`);
 for(const theme of ['light','dark'])for(const view of ['settings','dashboard'])for(const provider of ['Claude','Codex'])for(const width of [480,360]){
  await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:theme},{name:'prefers-reduced-motion',value:'reduce'}]});
  await cdp('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:2,mobile:false});
  await cdp('Page.navigate',{url:`http://127.0.0.1:1597/?view=${view}&scenario=multi&codex=yes&codexAccounts=multi&clock=controlled&platform=macos`});await sleep(700);
  if(view==='settings'){await click('#tab-providers');await sleep(200);}
  const key=`${view}-${provider}-${width}-${theme}`;
  const before=await measure();await shot(`${tag}-${key}-closed`);
  await click(`[aria-label="Ajouter un compte ${provider}"]`);await sleep(250);
  const open=await measure();await shot(`${tag}-${key}-open`);
  await cdp('Input.insertText',{text:'/Users/example/'+('long-profile-name-'.repeat(30))});await sleep(100);
  const typing=await measure();report[key]={before,open,typing};
  if(measureOnly)continue;
  assert.equal(open.input.h,32,key+' field height');assert.equal(open.margin,'0px',key+' margin');
  assert.equal(open.row.h,32,key+' row height');assert.equal(open.flow.h,54,key+' panel height');
  assert.ok(open.folder.right<=open.input.right && open.folder.x>open.input.x,key+' folder inside input suffix');assert.equal(open.folder.h,30);
  assert.equal(open.input.y+1,open.folder.y,key+' vertical alignment');
  assert.ok(await ev(`(()=>{const i=document.querySelector('.add-account__flow input');const b=document.querySelector('.add-account__choose');return i.getBoundingClientRect().right-parseFloat(getComputedStyle(i).paddingRight)<=b.getBoundingClientRect().left})()`),key+' text avoids icon');
  assert.equal(open.overflow,false,key+' overflow');assert.equal(typing.row.h,open.row.h);assert.equal(typing.flow.h,open.flow.h);assert.equal(typing.content.h,open.content.h);assert.equal(typing.window,open.window);
  await ev(`document.querySelector('.add-account__choose').focus()`);
  assert.equal(await ev(`document.activeElement.getAttribute('aria-label')`),'Choisir un dossier');
  await cdp('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await sleep(150);
  const closed=await measure();assert.equal(closed.flow,null);assert.equal(closed.content.h,before.content.h,key+' restored height');
  await cdp('Page.navigate',{url:`http://127.0.0.1:1597/?view=${view}&scenario=multi&codex=yes&codexAccounts=multi&clock=controlled&platform=linux`});await sleep(700);
  if(view==='settings'){await click('#tab-providers');await sleep(150);}
  await click(`[aria-label="Ajouter un compte ${provider}"]`);await sleep(200);
  const bare=await measure();report[key].bare=bare;
  assert.equal(bare.folder,null,key+' no native chooser');assert.equal(bare.row.h,32);assert.equal(bare.flow.h,54);
  assert.equal(await ev(`getComputedStyle(document.querySelector('.add-account__flow input')).paddingRight`),'8px',key+' no reserved suffix');
 }
 fs.writeFileSync(path.join(out,`add-${tag}.json`),JSON.stringify(report,null,2));
 console.log(measureOnly?'ADD_GEOMETRY_MEASURED':'ADD_GEOMETRY_PASS',Object.keys(report).length);
 assert.deepEqual(errors,[]);
}finally{clearTimeout(kill);ws?.close();chrome.kill();server.close();}
