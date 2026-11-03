// Settings → Fournisseurs (Codex AND Claude): minimal account detail, automatic detection, + button.
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
 const calls=()=>ev(`window.__fixture.calls.filter(c=>!c.command.startsWith('get_')&&!c.command.startsWith('choose_')&&c.command!=='fit_settings_window').map(c=>[c.command,c.payload])`);
 // helpers evaluated in the page: one provider section
 const sec=p=>`document.getElementById('${p}-heading').closest('section')`;
 const rows=p=>ev(`[...${sec(p)}.querySelectorAll('.claude-accounts__row')].map(r=>r.innerText.replace(/\\s+/g,' ').trim())`);
 const click=(sel)=>ev(`document.querySelector(${JSON.stringify(sel)}).click()`);
 const base='view=settings&scenario=multi&codex=yes&codexAccounts=multi&clock=controlled&platform=macos&layout=compact';
 const one='view=settings&scenario=one&codex=yes&codexAccounts=one&clock=controlled&platform=macos&layout=compact';

 // ---- screenshots, before / after, both schemes ----
 for(const scheme of ['light','dark'])for(const which of Object.keys(roots)){
  for(const [name,query] of [['one',one],['multi',base]]){
   await open(which,query,scheme);report[`${name}-${which}-${scheme}`]={rows:{codex:await rows('codex'),claude:await rows('claude')}};await shot(`settings-${name}-${which}-${scheme}`);
  }
  await open(which,base,scheme);
  await ev(`document.querySelector('[aria-label="Détail Codex Travail"]').click();document.querySelector('[aria-label="Détail Travail"]').click()`);await sleep(250);await shot(`settings-detail-${which}-${scheme}`);
 }

 // ---- assertions on the new build ----
 await open('after',base);
 // 1. detected homes are plain accounts: no "Détecté", no "Suivre", states per account
 const codexRows=await rows('codex');report.codexRows=codexRows;
 assert.equal(codexRows.length,5);assert.ok(codexRows.every(r=>!/Détecté|Suivre|Ne plus suivre/.test(r)),codexRows.join('|'));
 assert.match(codexRows[2],/Perso.*Non connecté/);assert.match(codexRows[3],/Alias.*Même compte/);assert.match(codexRows[4],/detecte.*Connecté/);
 // 2. detail of a followed account = path + Supprimer, nothing else, inline under the row
 await ev(`document.querySelector('[aria-label="Détail Codex Travail"]').focus()`);await key('Enter','Enter',13,'\r');
 const detail=await ev(`(()=>{const d=document.querySelector('#codex-detail-profile\\\\:travail');const li=d.closest('li');const code=d.querySelector('code');return {parentIsRow:li.classList.contains('claude-accounts__row'),children:[...d.children].map(c=>c.tagName+':'+c.textContent.trim()),text:d.innerText.replace(/\\s+/g,' ').trim(),title:code.title,select:getComputedStyle(code).userSelect,ellipsis:getComputedStyle(code).textOverflow,belowRow:d.getBoundingClientRect().top>=li.querySelector('.claude-accounts__who').getBoundingClientRect().bottom-1,count:document.querySelectorAll('.claude-accounts__detail').length,dialogs:document.querySelectorAll('dialog[open],[role=dialog]').length,expanded:document.querySelector('[aria-label="Détail Codex Travail"]').getAttribute('aria-expanded')}})()`);
 report.detail=detail;
 assert.deepEqual(detail.children,['CODE:/fixtures/.codex-travail','BUTTON:Supprimer Travail']);
 assert.equal(detail.text,'/fixtures/.codex-travail Supprimer Travail');assert.equal(detail.title,'/fixtures/.codex-travail');
 assert.equal(detail.select,'text');assert.equal(detail.ellipsis,'ellipsis');assert.ok(detail.parentIsRow&&detail.belowRow);assert.equal(detail.count,1);assert.equal(detail.dialogs,0);assert.equal(detail.expanded,'true');
 // 3. primary: path only, no Supprimer
 await click('[aria-label="Détail Codex Principal"]');await sleep(150);
 const primary=await ev(`(()=>{const d=document.querySelector('#codex-detail-primary');return {children:[...d.children].map(c=>c.tagName),buttons:d.querySelectorAll('button').length,open:document.querySelectorAll('.claude-accounts__detail').length}})()`);
 assert.deepEqual(primary.children,['CODE']);assert.equal(primary.buttons,0);
 // 4. keyboard: Escape closes the detail and returns focus to its chevron
 await ev(`document.querySelector('[aria-label="Détail Codex Travail"]').click()`);await sleep(100);
 await ev(`document.querySelector('#codex-detail-profile\\\\:travail button').focus()`);await key('Escape','Escape',27);await sleep(100);
 const esc=await ev(`({open:document.querySelectorAll('.claude-accounts__detail').length,focus:document.activeElement.getAttribute('aria-label')})`);
 assert.deepEqual(esc,{open:0,focus:'Détail Codex Travail'});
 // 5. + : one icon button per provider, bottom right, accessible names, no "Ajouter un compte" line
 const plus=await ev(`(()=>{const f=p=>{const s=document.getElementById(p+'-heading').closest('section');const b=s.querySelector('.claude-accounts__add');const l=s.querySelector('.claude-accounts__list').getBoundingClientRect();const r=b.getBoundingClientRect();return {label:b.getAttribute('aria-label'),text:b.textContent.trim(),right:Math.round(l.right-r.right),below:r.top>=l.bottom-1,w:Math.round(r.width),h:Math.round(r.height),svg:!!b.querySelector('svg'),line:/Ajouter un compte/.test(s.innerText)}};return {codex:f('codex'),claude:f('claude')}})()`);
 report.plus=plus;
 for(const [p,label] of [['codex','Ajouter un compte Codex'],['claude','Ajouter un compte Claude']]){const v=plus[p];assert.equal(v.label,label);assert.equal(v.text,'');assert.ok(v.svg&&v.below&&v.right<=2&&v.w>=22&&v.h>=22,JSON.stringify(v));assert.equal(v.line,false);}
 // 6. Supprimer (Codex): stops tracking only; command carries the account id; focus lands on the +; stays gone after a refresh
 await open('after',base+'&pickCodex=/fixtures/.codex-travail&pick=/fixtures/.claude-work');
 await click('[aria-label="Détail Codex Travail"]');await sleep(100);
 await ev(`document.querySelector('#codex-detail-profile\\\\:travail button').focus()`);await key('Enter','Enter',13,'\r');await sleep(500);
 let c=await calls();assert.deepEqual(c,[['untrack_codex_profile',{id:'profile:travail'}]]);
 const after=await rows('codex');assert.equal(after.length,4);assert.ok(!after.some(r=>/Travail/.test(r)));
 assert.equal(await ev(`document.activeElement.getAttribute('aria-label')`),'Ajouter un compte Codex');
 assert.equal(await ev(`window.__fixture.prefs.codexAccounts.excluded.join()`),'profile:travail');
 await ev(`window.__fixture.emit()`);await sleep(400);assert.equal((await rows('codex')).length,4);
 // 7. + routes to the right provider: Codex + → add_codex_profile only; the removed home is back
 await click('[aria-label="Ajouter un compte Codex"]');await sleep(500);
 c=await calls();assert.deepEqual(c.slice(1),[['add_codex_profile',{path:'/fixtures/.codex-travail'}]]);
 assert.equal((await rows('codex')).length,5);
 // Claude: Supprimer then + restores; commands never cross providers
 await click('[aria-label="Détail Travail"]');await sleep(100);
 const claudeDetail=await ev(`(()=>{const d=document.querySelector('[aria-label="Détail Travail"]').closest('li').querySelector('.claude-accounts__detail');return [...d.children].map(x=>x.tagName+':'+x.textContent.trim())})()`);
 assert.deepEqual(claudeDetail,['CODE:/fixtures/.claude-work','BUTTON:Supprimer Travail']);
 await ev(`document.querySelector('[aria-label="Détail Travail"]').closest('li').querySelector('.claude-accounts__remove').click()`);await sleep(500);
 assert.equal((await rows('claude')).length,1);
 assert.equal(await ev(`document.activeElement.getAttribute('aria-label')`),'Ajouter un compte Claude');
 await click('[aria-label="Ajouter un compte Claude"]');await sleep(500);
 c=await calls();assert.deepEqual(c.slice(2).map(x=>x[0]),['untrack_claude_profile','add_claude_profile']);
 // (the fixture folder chooser returns the removed path for Claude through ?pick, which is /fixtures/.claude-work)
 assert.equal((await rows('claude')).length,2);
 assert.ok(c.slice(2).every(x=>!/codex/.test(x[0])),'no Codex command from the Claude +');
 // 8. without a native folder chooser (not macOS) the + opens the absolute-path field
 await open('after',base.replace('platform=macos','platform=linux'));
 await click('[aria-label="Ajouter un compte Codex"]');await sleep(300);
 const manual=await ev(`({focus:document.activeElement.getAttribute('aria-label'),open:!!document.querySelector('.claude-accounts__manual')})`);
 assert.deepEqual(manual,{focus:'Chemin absolu d’un dossier CODEX_HOME',open:true});
 // 9. single account: principal alone has the path, never a Supprimer
 await open('after',one);await click('[aria-label="Détail Codex Principal"]');await click('[aria-label="Détail Personnel"]');await sleep(150);
 const solo=await ev(`[...document.querySelectorAll('.claude-accounts__detail')].map(d=>[...d.children].map(c=>c.tagName).join())`);
 assert.deepEqual(solo,['CODE','CODE']);
 await shot('settings-one-detail-after-light');
 assert.deepEqual(errors,[]);
 fs.writeFileSync(path.join(out,'report.json'),JSON.stringify(report,null,2));console.log('ACCOUNTS_DETAIL_PASS',Object.keys(report).length);
}finally{clearTimeout(kill);ws?.close();chrome.kill();servers.forEach(s=>s.close());}
