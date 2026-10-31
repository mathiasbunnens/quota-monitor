// Offline browser fixture: actual React views/styles, explicitly mocked Tauri IPC.
// No app identity, user settings, Claude process, updater or notification is used.
import { build } from 'vite';
import react from '@vitejs/plugin-react';
import fs from 'node:fs';
import path from 'node:path';
const out = path.resolve(process.argv[2] || 'evidence/claude-multi/renderer');
fs.mkdirSync(out, { recursive: true });
const mock = `
const query = new URLSearchParams(location.search);
const listeners = new Set();
let clock = Date.parse('2026-10-31T10:00:00Z');
const timers = new Map();
if(query.get('clock')==='controlled') {
 Date.now=()=>clock;
 const timeout=window.setTimeout.bind(window), interval=window.setInterval.bind(window), clear=window.clearInterval.bind(window);
 window.setTimeout=(fn,ms,...args)=>ms===2000?0:timeout(fn,ms,...args); // Hold cache polling after initial snapshot.
 window.setInterval=(fn,ms,...args)=>{if(ms!==60000)return interval(fn,ms,...args);const id=1000+timers.size;timers.set(id,fn);return id;};
 window.clearInterval=id=>{timers.delete(id);clear(id);};
}
const base = Date.now();
const metric = (id, used, label, minutes=id==='five_hour'?210:4800) => ({id:'claude:'+id,label,usedPercent:used,availability:used===null?'not_reported':'available',resetAt:new Date(base+minutes*60000).toISOString(),periodDurationSeconds:id==='five_hour'?18000:604800,exhausted:used===100});
const snapshot = (used, weekly, plan) => ({provider:'claude',schemaVersion:1,enabled:true,configured:true,state:'ready',source:null,observedAt:'2026-10-31T10:00:00Z',providerFetchedAt:null,message:'',errorKind:null,metrics:[metric('five_hour',used,'Limite 5 heures'),metric('seven_day',weekly,'Limite hebdomadaire')],capability:{refresh:'on_demand'},planType:plan,extraUsage:null});
const profiles = [{id:'personal',path:'/fixtures/.claude',name:'Personnel',visible:true,primary:true,available:true,refreshing:false,snapshot:snapshot(27,14,'pro')},{id:'work',path:'/fixtures/.claude-work',name:'Travail',visible:true,primary:false,available:true,refreshing:false,snapshot:snapshot(65,80,'max')}];
profiles[1].snapshot.metrics[0].resetAt=new Date(base+42*60000).toISOString();
profiles[1].snapshot.metrics[1].resetAt=new Date(base+2940*60000).toISOString();
const scenario = query.get('scenario') || 'multi';
if (scenario === 'edge') {profiles[0].snapshot.metrics = [metric('five_hour',100,'Limite 5 heures'),metric('seven_day',null,'Limite hebdomadaire'),{...metric('seven_day',15,'Limite hebdomadaire Fable'),id:'claude:weekly_fable'}];profiles[0].snapshot.extraUsage={enabled:true,limitReached:true};profiles[1].snapshot.state='error';profiles[1].snapshot.metrics=[];profiles[1].snapshot.message='Lecture Claude impossible : vérifiez la connexion.';}
if (scenario === 'fable') for (const [i,p] of profiles.entries()) p.snapshot.metrics.push({...metric('seven_day',i?91:40,'Limite hebdomadaire Fable',4800+1200),id:'claude:weekly_fable'});
let prefs = {codexEnabled:query.get('codex')==='yes',claudeEnabled:true,fableEnabled:query.get('fable')!=='off',indicator:'claude',displayMode:'menubar',ntfy:{enabled:false,serverUrl:'https://ntfy.sh',topic:''},claudeAccounts:{mode:scenario==='single'?'single':'multi',selectedId:null,profiles:profiles.map(({id,path,name,visible})=>({id,path,name,visible}))}};
if(scenario==='none') prefs.claudeAccounts.profiles.forEach(p=>p.visible=false);
if(scenario==='one-visible') prefs.claudeAccounts.profiles[1].visible=false;
const accounts = () => profiles.map(a=>{const saved=prefs.claudeAccounts.profiles.find(p=>p.id===a.id);return {...a,name:saved?.name||a.name,visible:prefs.claudeEnabled&&(prefs.claudeAccounts.mode==='multi'?(saved?.visible??a.primary):a.id===(prefs.claudeAccounts.selectedId||'personal'))};});
const emit = () => { for(const listener of listeners) listener(); };
window.__fixture = {get prefs(){return prefs},get accounts(){return accounts()},calls:[],emit,get minuteTimers(){return timers.size},advanceMinute(){clock+=60000;for(const fn of timers.values())fn();}};
export const listen = async (_event, cb) => {listeners.add(cb);return ()=>listeners.delete(cb)};
export async function invoke(command, payload) {
 window.__fixture.calls.push({command,payload});
 switch(command){
 case 'get_platform':return query.get('platform')||'linux';
 case 'get_preferences':return structuredClone(prefs);
 case 'set_preferences':prefs=structuredClone(payload.prefs);emit();return structuredClone(prefs);
 case 'get_claude_accounts':return structuredClone(accounts());
 case 'get_claude_snapshot':return accounts().find(a=>a.visible)?.snapshot||profiles[0].snapshot;
 case 'get_quota_snapshots':return prefs.codexEnabled?[{model:'codex',remaining:63,limit:100,period:'five-hour',resetAt:'2030-01-01T12:00:00Z',checkedAt:'2026-10-31T10:00:00Z',source:'codex',confidence:'high'}]:[];
 case 'get_codex_status':return {phase:'ready',message:'',planType:'plus',refreshSettings:{customSeconds:null},refreshSeconds:60,availableResets:null};
 case 'add_claude_profile':throw new Error('Fixture: dossier inexistant');
 case 'fit_settings_window': case 'request_claude_refresh': case 'request_quota_refresh': return null;
 default: throw new Error('Blocked external action in fixture: '+command);
 }
}
`;
fs.writeFileSync(path.join(out,'fixture.tsx'),`import React from 'react';import{createRoot}from'react-dom/client';import Dashboard from ${JSON.stringify(path.resolve('src/pages/Dashboard/Dashboard.tsx'))};import Settings from ${JSON.stringify(path.resolve('src/features/settings/Settings.tsx'))};const q=new URLSearchParams(location.search);createRoot(document.getElementById('root')).render(q.get('view')==='settings'?<Settings/>:<Dashboard/>);`);
fs.writeFileSync(path.join(out,'index.html'),'<!doctype html><html lang="fr"><meta charset="utf-8"><title>Offline Claude fixture</title><style>*{box-sizing:border-box}body{margin:0}</style><div id="root"></div><script type="module" src="./fixture.tsx"></script></html>');
await build({configFile:false,root:out,plugins:[react(),{name:'fixture-ipc',enforce:'pre',resolveId(id){if(/^@tauri-apps\/api\/(core|event)$/.test(id))return '\0fixture-ipc'},load(id){if(id==='\0fixture-ipc')return mock}}],build:{outDir:path.join(out,'site'),emptyOutDir:true}});
console.log('RENDERER_FIXTURE_BUILT '+out);
