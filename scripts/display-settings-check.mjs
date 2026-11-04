// Réglages → Affichage: offline captures + geometry. SITE = built renderer
// fixture (scripts/claude-renderer-fixture.mjs <dir> → <dir>/site), SHOTS =
// output dir, TAG = file prefix, MEASURE_ONLY=1 records without asserting
// (baseline). Headless shell over CDP: no window, no focus stolen.
import {spawn} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';
const out=path.resolve(process.env.SHOTS||'node_modules/.cache/display-shots');fs.mkdirSync(out,{recursive:true});
const site=path.resolve(process.env.SITE||'node_modules/.cache/disp-after/site');
const measureOnly=!!process.env.MEASURE_ONLY;const tag=process.env.TAG||'after';
const types={js:'text/javascript',css:'text/css',html:'text/html',json:'application/json',svg:'image/svg+xml',png:'image/png'};
const server=http.createServer((req,res)=>{const u=new URL(req.url,'http://x').pathname;const f=path.join(site,u.endsWith('/')?u+'index.html':u);if(!f.startsWith(site)||!fs.existsSync(f)){res.writeHead(404);return res.end();}res.setHeader('Content-Type',types[f.split('.').pop()]||'application/octet-stream');res.end(fs.readFileSync(f));}).listen(1598,'127.0.0.1');
const shell=process.env.CHROME_SHELL||process.env.HOME+'/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome=spawn(shell,['--remote-debugging-port=9398','--user-data-dir='+path.join(out,'.profile'),'--no-first-run','--hide-scrollbars','about:blank'],{stdio:'ignore'});
const kill=setTimeout(()=>{chrome.kill('SIGKILL');process.exit(2);},200000);
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let ws;const errors=[];const report={};
const near=(a,b,eps=0.6)=>Math.abs(a-b)<=eps;
try{
 let target;for(let i=0;i<50&&!target;i++){await sleep(200);try{target=(await(await fetch('http://127.0.0.1:9398/json')).json()).find(t=>t.type==='page');}catch{}}
 ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let id=0;const pending=new Map();ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}else if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.text);});
 const cdp=(method,params={})=>new Promise((resolve,reject)=>{const n=++id;pending.set(n,m=>m.error?reject(Error(m.error.message)):resolve(m.result));ws.send(JSON.stringify({id:n,method,params}));});
 const ev=async e=>(await cdp('Runtime.evaluate',{expression:e,returnByValue:true,awaitPromise:true})).result.value;
 const shot=async name=>fs.writeFileSync(path.join(out,name+'.png'),Buffer.from((await cdp('Page.captureScreenshot',{format:'png'})).data,'base64'));
 await cdp('Page.enable');await cdp('Runtime.enable');
 const key=async(k,code,vk)=>{await cdp('Input.dispatchKeyEvent',{type:'keyDown',key:k,code,windowsVirtualKeyCode:vk});await cdp('Input.dispatchKeyEvent',{type:'keyUp',key:k,code,windowsVirtualKeyCode:vk});};
 const measure=()=>ev(`(()=>{const panel=document.querySelector('#panel-menubar');const r=x=>{const a=x.getBoundingClientRect();return {x:+a.x.toFixed(2),y:+a.y.toFixed(2),w:+a.width.toFixed(2),h:+a.height.toFixed(2),right:+a.right.toFixed(2),bottom:+a.bottom.toFixed(2)}};
  const pr=panel.getBoundingClientRect();
  const groups=[...panel.querySelectorAll('[role=radiogroup]')].map(g=>({name:g.getAttribute('aria-label'),rect:r(g),
    row:g.closest('.prefs-row')?r(g.closest('.prefs-row')):null,
    label:g.closest('.prefs-row')?.querySelector('.prefs-row__label')?.textContent??null,
    labelRect:g.closest('.prefs-row')?.querySelector('.prefs-row__label')?r(g.closest('.prefs-row').querySelector('.prefs-row__label')):null,
    segments:[...g.querySelectorAll('label')].map(l=>{const t=l.querySelector('span:last-child');const i=l.querySelector('input');const lb=l.getBoundingClientRect(),first=(l.querySelector('.prefs-choices__icon')||t).getBoundingClientRect(),tb=t.getBoundingClientRect();return {text:l.textContent.trim(),rect:r(l),checked:i.checked,slack:+Math.min(first.x-lb.x,lb.right-tb.right).toFixed(2),clipped:t.scrollWidth>t.clientWidth+0.5||l.scrollWidth>l.clientWidth+0.5,icon:l.querySelector('svg')?r(l.querySelector('svg')):null}})}));
  const outside=[...panel.querySelectorAll('*')].filter(n=>{const a=n.getBoundingClientRect();return a.width>0&&(a.right>pr.right+0.5||a.x<pr.x-0.5)}).length;
  return {panel:r(panel),groupCards:panel.querySelectorAll('.prefs-group').length,groups,outside,
    docOverflow:document.documentElement.scrollWidth>innerWidth,content:r(document.querySelector('main.prefs'))}})()`);
 for(const theme of ['light','dark'])for(const width of [600,480,360])for(const mode of ['menubar','app']){
  await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:theme},{name:'prefers-reduced-motion',value:'reduce'}]});
  await cdp('Emulation.setDeviceMetricsOverride',{width,height:640,deviceScaleFactor:2,mobile:false});
  await cdp('Page.navigate',{url:`http://127.0.0.1:1598/?view=settings&scenario=multi&codex=yes&codexAccounts=multi&clock=controlled&platform=macos`});await sleep(700);
  await ev(`document.querySelector('#tab-menubar').click()`);await sleep(200);
  if(mode==='app'){await ev(`document.querySelector('input[name=displayMode][value=app]').click()`);await sleep(250);}
  const name=`${width}-${mode}-${theme}`;const m=await measure();report[name]=m;await shot(`${tag}-${name}`);
  if(measureOnly)continue;
  assert.equal(m.groupCards,1,name+' one group');
  assert.equal(m.groups.length,mode==='app'?2:3,name+' rows');
  assert.equal(m.docOverflow,false,name+' doc overflow');assert.equal(m.outside,0,name+' element outside panel');
  const segs=m.groups.flatMap(g=>g.segments);
  for(const s of segs){assert.ok(near(s.rect.h,segs[0].rect.h),name+' segment height '+s.text);assert.equal(s.clipped,false,name+' clipped '+s.text);assert.ok(s.slack>=6,name+' inner margin '+s.text+' '+s.slack);}
  for(const g of m.groups){
   assert.ok(near(g.rect.right,m.groups[0].rect.right),name+' control right edge '+g.name);
   assert.ok(near(g.rect.x,m.groups[0].rect.x),name+' control left edge '+g.name);
   assert.ok(near(g.rect.h,m.groups[0].rect.h),name+' control height '+g.name);
   for(const s of g.segments)assert.ok(near(s.rect.w,g.segments[0].rect.w,1),name+' equal segments '+g.name);
   assert.ok(g.label&&g.labelRect,name+' visible row label');
   assert.ok(g.name.toLowerCase().includes(g.label.toLowerCase().replace('’',"’")),name+' label in name '+g.name);
   if(width>=480)assert.ok(g.labelRect.right<=g.rect.x,name+' label left of control');
   else assert.ok(g.labelRect.bottom<=g.rect.y+0.5,name+' label above control');
   for(const s of g.segments)assert.ok(s.rect.y>=g.row.y&&s.rect.bottom<=g.row.bottom,name+' segment within row');
  }
  for(let i=1;i<m.groups.length;i++)assert.ok(m.groups[i].row.y>=m.groups[i-1].row.bottom-0.5,name+' rows stacked');
  assert.deepEqual(m.groups.map(g=>g.name),mode==='app'?['Mode d’affichage','Présentation']:['Mode d’affichage','Indicateur de la barre des menus (quota 5 heures)','Présentation']);
 }
 if(!measureOnly){
  // Keyboard: arrows move inside a radiogroup and persist through set_preferences.
  await cdp('Emulation.setDeviceMetricsOverride',{width:480,height:640,deviceScaleFactor:2,mobile:false});
  await cdp('Page.navigate',{url:`http://127.0.0.1:1598/?view=settings&scenario=multi&codex=yes&codexAccounts=multi&clock=controlled&platform=macos`});await sleep(700);
  await ev(`document.querySelector('#tab-menubar').click()`);await sleep(200);
  const checked=n=>ev(`document.querySelector('input[name=${n}]:checked')?.value`);
  const before=await checked('layout');
  await ev(`document.querySelector('input[name=layout]:checked').focus()`);
  await key('ArrowRight','ArrowRight',39);await sleep(300);
  const afterKey=await checked('layout');assert.notEqual(afterKey,before,'arrow key changes layout');
  assert.equal(await ev(`document.activeElement.name`),'layout','focus stays in the group');
  assert.ok(await ev(`!!document.querySelector('label:has(input[name=layout]:focus-visible)')`),'focus ring on segment');
  await shot(`${tag}-480-focus-light`);
  await ev(`document.querySelector('input[name=indicator]:checked').focus()`);
  await key('ArrowRight','ArrowRight',39);await sleep(300);
  assert.equal(await checked('indicator'),'both','indicator arrow key');
  report.keyboard={before,afterKey,indicator:'both'};
 }
 fs.writeFileSync(path.join(out,`display-${tag}.json`),JSON.stringify(report,null,2));
 console.log(measureOnly?'DISPLAY_MEASURED':'DISPLAY_GEOMETRY_PASS',Object.keys(report).length);
 assert.deepEqual(errors,[]);
}finally{clearTimeout(kill);ws?.close();chrome.kill();server.close();}
