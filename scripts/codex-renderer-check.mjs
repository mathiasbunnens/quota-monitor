// Offline headless CDP assertions and captures. No user-facing window.
import {spawn} from 'node:child_process';
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';
const headingsOnly=process.env.SETTINGS_HEADINGS==='1';
const out=path.resolve('evidence/t_33379e53/'+(headingsOnly?'settings-headings':'renderer'));fs.mkdirSync(out,{recursive:true});
const roots=[headingsOnly?'codex-headings-before':'codex-before','codex-after','codex-compact'].map(n=>path.resolve('node_modules/.cache',n,'site'));
const servers=roots.map((root,index)=>http.createServer((req,res)=>{const file=path.resolve(root,'.'+new URL(req.url,'http://x').pathname.replace(/\/$/,'/index.html'));if(!file.startsWith(root+path.sep)||!fs.existsSync(file)){res.writeHead(404);return res.end();}res.setHeader('Content-Type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.json')?'application/json':'text/html');fs.createReadStream(file).pipe(res);}).listen(1461+index,'127.0.0.1'));
const shell=process.env.HOME+'/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome=spawn(shell,['--remote-debugging-port=9353','--user-data-dir='+path.join(out,'.profile'),'--no-first-run','--hide-scrollbars','--use-mock-keychain','--password-store=basic','--disable-gpu-vsync','--disable-frame-rate-limit','about:blank'],{stdio:'ignore'});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let ws;const results=[];const errors=[];
try{
 let target;for(let i=0;i<50&&!target;i++){await sleep(200);try{target=(await(await fetch('http://127.0.0.1:9353/json')).json()).find(t=>t.type==='page');}catch{}}
 assert.ok(target,'headless CDP ready');ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let id=0;const pending=new Map();ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){pending.get(m.id)?.(m);pending.delete(m.id);}else if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.text);});
 const cdp=(method,params={})=>new Promise((resolve,reject)=>{const n=++id;pending.set(n,m=>m.error?reject(Error(m.error.message)):resolve(m.result));ws.send(JSON.stringify({id:n,method,params}));});
 const evaluate=async expression=>(await cdp('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true})).result.value;
 await cdp('Page.enable');await cdp('Runtime.enable');
 const open=async(index,query,width,scheme)=>{await cdp('Emulation.setDeviceMetricsOverride',{width,height:840,deviceScaleFactor:2,mobile:false});await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:scheme},{name:'prefers-reduced-motion',value:'reduce'}]});await cdp('Page.navigate',{url:`http://127.0.0.1:${1461+index}/?${query}`});await sleep(500);};
 const capture=async name=>fs.writeFileSync(path.join(out,name+'.png'),Buffer.from((await cdp('Page.captureScreenshot',{format:'png'})).data,'base64'));
 const geometry=()=>evaluate(`(()=>{const all=s=>[...document.querySelectorAll(s)];const rect=e=>{const r=e.getBoundingClientRect();return {height:r.height,top:r.top,bottom:r.bottom}};return {overflow:document.documentElement.scrollWidth>innerWidth,short:all('.provider-card__window--short,.compact-pair__track--short').map(rect),week:all('.provider-card__window--week,.compact-pair__track--week').map(rect),meters:all('[role=progressbar],[role=meter]').length,text:document.body.innerText}})()`);
 if(!headingsOnly){
 for(const scheme of ['light','dark'])for(const both of [false,true])for(const index of [0,1]){
  const query=`scenario=one&clock=controlled&layout=compact&codex=yes${both?'':'&claude=off'}`;
  await open(index,query,640,scheme);const state=await geometry();assert.equal(state.overflow,false);if(index===1){assert.equal(state.short.length,1+Number(both));assert.equal(state.week.length,1+Number(both));assert.equal(state.short[0].height,7);assert.equal(state.week[0].height,3);assert.equal(state.week[0].top-state.short[0].bottom,2);}
  const name=`dashboard-${both?'both':'codex'}-${index?'after':'before'}-${scheme}`;await capture(name);results.push({name,...state});
 }
 for(const scheme of ['light','dark'])for(const scenario of ['weekly','edge','empty']){await open(1,`scenario=one&codex=yes&claude=off&layout=compact&codexState=${scenario}`,360,scheme);const state=await geometry();assert.equal(state.overflow,false);assert.equal(state.meters,scenario==='empty'?0:scenario==='weekly'?1:3);if(scenario==='weekly'){assert.equal(state.short.length,0);assert.equal(await evaluate(`document.querySelector('.provider-card__window--week').getBoundingClientRect().y-document.querySelector('.provider-card__paired-bars').getBoundingClientRect().y`),9);}results.push({name:'edge-'+scenario+'-'+scheme,...state});}
 for(const scheme of ['light','dark'])for(const model of ['codex-compact','codex-classic','codex-both-compact','codex-weekly-only']){
  await open(2,`model=${model}`,324,scheme);const state=await geometry();assert.equal(state.overflow,false);if(model==='codex-compact'){assert.equal(state.short[0].height,5);assert.equal(state.week[0].height,3);assert.equal(state.week[0].top-state.short[0].bottom,2);}
  if(model==='codex-weekly-only'){assert.equal(state.short.length,0);assert.equal(state.meters,2);assert.equal(await evaluate(`document.querySelectorAll('.compact-pair [role="meter"]').length`),1);assert.equal(await evaluate(`document.querySelector('.compact-pair__track--week').getBoundingClientRect().y-document.querySelector('.compact-pair__bars').getBoundingClientRect().y`),7);}
  await capture(`compact-${model}-${scheme}`);results.push({name:`compact-${model}-${scheme}`,...state});
 }
 await open(1,'view=settings&scenario=one&codex=yes&layout=compact&platform=macos',480,'light');
 await evaluate(`document.getElementById('tab-menubar').click()`);await sleep(150);await capture('settings-light');
 await evaluate(`document.querySelector('input[name=layout][value=compact]').focus()`);
 const key=async(k,code)=>{await cdp('Input.dispatchKeyEvent',{type:'keyDown',key:k,code:k,windowsVirtualKeyCode:code});await cdp('Input.dispatchKeyEvent',{type:'keyUp',key:k,code:k,windowsVirtualKeyCode:code});await sleep(180);};
 await key('ArrowRight',39);assert.equal(await evaluate('window.__fixture.prefs.layout'),'classic');assert.equal(await evaluate('window.__fixture.prefs.claudeLayout'),'classic');
 await key('ArrowLeft',37);assert.equal(await evaluate('window.__fixture.prefs.layout'),'compact');
 const mutations=await evaluate(`window.__fixture.calls.filter(c=>!c.command.startsWith('get_')&&c.command!=='fit_settings_window').map(c=>c.command)`);assert.deepEqual(mutations,['set_preferences','set_preferences']);
 await cdp('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:'dark'}]});await capture('settings-dark');
 await open(1,'scenario=one&clock=controlled&codex=yes&claude=off&layout=compact',640,'light');const text=await evaluate('document.body.innerText');await evaluate('window.__fixture.advanceMinute()');await sleep(150);assert.notEqual(await evaluate('document.body.innerText'),text);assert.equal(await evaluate(`window.__fixture.calls.filter(c=>c.command==='request_quota_refresh').length`),0);
 assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'results.json'),JSON.stringify({results,keyboard:{codexLayout:'compact',claudeUnchanged:true,mutations},localCountdown:true,errors},null,2));console.log(`CODEX_RENDERER_PASS ${results.length} scenes; keyboard/persistence IPC, geometry, local countdown; no exceptions`);
 }else{
 const failures=[];
 for(const scheme of ['light','dark'])for(const index of [0,1])for(const tab of ['providers','menubar','refresh','notifications']){
  await open(index,'view=settings&scenario=one&codex=yes&layout=compact&platform=macos',480,scheme);
  await evaluate(`document.getElementById('tab-${tab}').click()`);await sleep(150);
  if(tab==='providers')await evaluate(`document.querySelector('[aria-label="Compte Codex"]').click()`);
  const state=await evaluate(`(()=>{const panel=document.getElementById('panel-${tab}');const style=e=>{const s=getComputedStyle(e);const r=e.getBoundingClientRect();return {text:e.textContent,size:s.fontSize,weight:s.fontWeight,lineHeight:s.lineHeight,transform:s.textTransform,left:r.left,width:r.width,height:r.height,clipped:e.scrollWidth>e.clientWidth+1}};return {groups:[...panel.querySelectorAll('.prefs-group > h2,.prefs-group__head h2,.prefs-group__legend')].map(style),options:[...panel.querySelectorAll('h3,.codex-connection h2')].map(style),overflow:document.documentElement.scrollWidth>innerWidth}})()`);
  const name=tab+'-'+(index?'after':'before')+'-'+scheme;await capture(name);results.push({name,...state});
  if(index){if(!state.groups.length)failures.push(name+': no group title');for(const h of state.groups)if(h.size!=='13px'||h.weight!=='600'||h.transform!=='none'||h.clipped)failures.push(name+': '+JSON.stringify(h));if(state.overflow)failures.push(name+': overflow');}
 }
 assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'results.json'),JSON.stringify({results,failures,errors},null,2));assert.deepEqual(failures,[]);console.log(`SETTINGS_HEADINGS_PASS ${results.length} comparable tab/theme captures`);
 }
}finally{ws?.close();chrome.kill();servers.forEach(s=>s.close());}
