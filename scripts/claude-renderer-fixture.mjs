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
if (query.get('extra')==='yes') profiles[0].snapshot.extraUsage={enabled:true,limitReached:false};
if (scenario === 'edge') {profiles[0].snapshot.metrics = [metric('five_hour',100,'Limite 5 heures'),metric('seven_day',null,'Limite hebdomadaire'),{...metric('seven_day',15,'Limite hebdomadaire Fable'),id:'claude:weekly_fable'}];profiles[0].snapshot.extraUsage={enabled:true,limitReached:true};profiles[1].snapshot.state='error';profiles[1].snapshot.metrics=[];profiles[1].snapshot.message='Lecture Claude impossible : vérifiez la connexion.';}
if (scenario === 'signed-out') Object.assign(profiles[1].snapshot, {state:'error',metrics:[],planType:null,errorKind:'not_signed_in',message:'Profil Claude non connecté. Connectez-vous avec CLAUDE_CONFIG_DIR=/fixtures/.claude-work claude auth login.'});
if (scenario === 'fable') for (const [i,p] of profiles.entries()) p.snapshot.metrics.push({...metric('seven_day',i?91:40,'Limite hebdomadaire Fable',4800+1200),id:'claude:weekly_fable'});
if (scenario === 'one') profiles.splice(1);
if (scenario === 'zero') { profiles.splice(1); Object.assign(profiles[0], {available:false}); Object.assign(profiles[0].snapshot, {state:'error',metrics:[],planType:null,message:'Dossier de profil Claude indisponible.'}); }
if (scenario === 'add-error') profiles[1].snapshot.planType = 'team';
let prefs = {codexEnabled:query.get('codex')==='yes',claudeEnabled:true,fableEnabled:query.get('fable')!=='off',indicator:'claude',displayMode:'menubar',ntfy:{enabled:false,serverUrl:'https://ntfy.sh',topic:''},claudeAccounts:{mode:['single','one','zero','add-error'].includes(scenario)?'single':'multi',selectedId:null,profiles:profiles.map(({id,path,name,visible})=>({id,path,name,visible}))},...(query.get('layout')?{layout:query.get('layout'),claudeLayout:query.get('layout'),codexLayout:query.get('layout')}:{})};
if(scenario==='none') prefs.claudeAccounts.profiles.forEach(p=>p.visible=false);
if(scenario==='one-visible') prefs.claudeAccounts.profiles[1].visible=false;
if(query.get('claude')==='off') prefs.claudeEnabled=false;
const codexRows = [
 {model:'codex',remaining:63,limit:100,period:'codex:codex:primary',windowMinutes:300,resetAt:new Date(base+95*60000).toISOString(),label:'Codex · 5 heures'},
 {model:'codex',remaining:85,limit:100,period:'codex:codex:secondary',windowMinutes:10080,resetAt:new Date(base+5760*60000).toISOString(),label:'Codex · 7 jours'},
];
if(query.get('codexState')==='weekly') codexRows.splice(0,1);
if(query.get('codexState')==='empty') codexRows.splice(0);
if(query.get('codexState')==='edge') {
 codexRows[0].remaining=0; codexRows[1].resetAt='';
 codexRows.push({...codexRows[1],period:'codex:review:secondary',model:'review',remaining:27,label:'Review · 7 jours'});
}
const accounts = () => profiles.map(a=>{const saved=prefs.claudeAccounts.profiles.find(p=>p.id===a.id);return {...a,name:saved?.name||a.name,visible:prefs.claudeEnabled&&(prefs.claudeAccounts.mode==='multi'?(saved?.visible??a.primary):a.id===(prefs.claudeAccounts.selectedId||'personal'))};});
// Codex homes (?codexAccounts=multi|one): synthetic rows only, never a real CODEX_HOME.
const codexRow=(r5,r7,m5=95)=>[{...codexRows[0],remaining:r5,resetAt:new Date(base+m5*60000).toISOString()},{...codexRows[1],remaining:r7}];
const codexHome=(name,extra={})=>({id:'profile:'+name,path:'/fixtures/.codex-'+name,name,visible:true,primary:false,tracked:true,available:true,refreshing:false,phase:'ready',message:'Quotas synchronisés directement avec Codex.',planType:'pro',lastChecked:null,rows:codexRow(18,64,42),duplicateOf:null,availableResets:1,...extra});
const codexHomes=query.get('codexAccounts')?[
 codexHome('Principal',{id:'primary',path:'~/.codex',primary:true,planType:'plus',rows:structuredClone(codexRows),availableResets:2}),
 codexHome('travail',{name:'Travail'}),
 codexHome('perso',{name:'Perso',phase:'signed_out',planType:null,rows:[],availableResets:null,message:'Non connecté. Connectez ce dossier dans un terminal : CODEX_HOME=/fixtures/.codex-perso codex login'}),
 codexHome('alias',{name:'Alias',planType:'plus',rows:structuredClone(codexRows),duplicateOf:'Principal',availableResets:2}),
 codexHome('detecte',{name:'.codex-detecte',tracked:false,visible:false,phase:'detected',rows:[],availableResets:null,planType:null,message:'Dossier détecté, non suivi.'}),
]:[];
if(query.get('codexAccounts')==='one') codexHomes.splice(1,3);
if(codexHomes.length) prefs.codexAccounts={mode:query.get('codexAccounts')==='multi'?'multi':'single',selectedId:null,profiles:codexHomes.filter(h=>h.tracked&&!h.primary).map(({id,path,name})=>({id,path,name,visible:true}))};
const codexAccounts=()=>codexHomes.map(h=>{const saved=prefs.codexAccounts?.profiles.find(p=>p.id===h.id);const multi=prefs.codexAccounts?.mode==='multi';return {...h,name:saved?.name||h.name,visible:prefs.codexEnabled&&h.tracked&&(multi?(h.primary?prefs.codexAccounts.primaryVisible!==false:saved?.visible??true):h.id===(prefs.codexAccounts?.selectedId||'primary'))};});
const emit = () => { for(const listener of listeners) listener(); };
window.__fixture = {get prefs(){return prefs},get accounts(){return accounts()},calls:[],emit,get minuteTimers(){return timers.size},advanceMinute(){clock+=60000;for(const fn of timers.values())fn();}};
export const listen = async (_event, cb) => {listeners.add(cb);return ()=>listeners.delete(cb)};
export async function invoke(command, payload) {
 window.__fixture.calls.push({command,payload});
 switch(command){
 case 'get_platform':return query.get('platform')||'linux';
 case 'get_preferences':return structuredClone(prefs);
 case 'set_preferences':prefs=structuredClone(payload.prefs);if(prefs.layout)prefs.claudeLayout=prefs.codexLayout=prefs.layout;emit();return structuredClone(prefs);
 case 'get_claude_accounts':return structuredClone(accounts());
 case 'get_claude_snapshot':return accounts().find(a=>a.visible)?.snapshot||profiles[0].snapshot;
 case 'get_quota_snapshots':return prefs.codexEnabled?structuredClone(codexRows):[];
 case 'get_codex_status':return {phase:'ready',message:'',planType:'plus',refreshSettings:{customSeconds:null},refreshSeconds:60,availableResets:2};
 case 'redeem_codex_reset':{const home=codexHomes.find(h=>h.id===payload.accountId);if(!home||!home.availableResets)throw new Error('Fixture: aucun compte');home.availableResets-=1;emit();return {outcome:'reset',quotasRefreshed:true};}
 case 'open_usage_details': case 'open_claude_usage': return null;
 case 'add_claude_profile':if(payload.path!=='/fixtures/.claude-work')throw new Error('Fixture: dossier inexistant');prefs={...prefs,claudeAccounts:{...prefs.claudeAccounts,mode:'multi',profiles:profiles.map(({id,path,name})=>({id,path,name,visible:true}))}};emit();return structuredClone(prefs);
 case 'follow_claude_profile':if(scenario==='add-error')throw new Error('« Travail » est déjà affiché. Choisissez un autre profil Claude Code.');prefs={...prefs,claudeAccounts:{...prefs.claudeAccounts,mode:'multi',profiles:profiles.map(({id,path,name})=>({id,path,name,visible:true}))}};emit();return structuredClone(prefs);
 case 'get_codex_accounts':return structuredClone(codexAccounts());
 case 'follow_codex_profile':case 'add_codex_profile':{const id=payload.id||'profile:new';codexHomes.forEach(h=>{if(h.id===id)h.tracked=true;});prefs={...prefs,codexAccounts:{mode:'multi',selectedId:null,profiles:codexHomes.filter(h=>h.tracked&&!h.primary).map(({id,path,name})=>({id,path,name,visible:true}))}};emit();return structuredClone(prefs);}
 case 'untrack_codex_profile':codexHomes.forEach(h=>{if(h.id===payload.id)h.tracked=false;});prefs={...prefs,codexAccounts:{...prefs.codexAccounts,profiles:(prefs.codexAccounts?.profiles||[]).filter(p=>p.id!==payload.id)}};emit();return structuredClone(prefs);
 case 'choose_codex_profile_folder':return null;
 case 'choose_claude_profile_folder':return query.get('pick')||null;
 case 'fit_settings_window': case 'request_claude_refresh': case 'request_quota_refresh': return null;
 default: throw new Error('Blocked external action in fixture: '+command);
 }
}
`;
fs.writeFileSync(path.join(out,'fixture.tsx'),`import React from 'react';import{createRoot}from'react-dom/client';import Dashboard from ${JSON.stringify(path.resolve('src/pages/Dashboard/Dashboard.tsx'))};import Settings from ${JSON.stringify(path.resolve('src/features/settings/Settings.tsx'))};const q=new URLSearchParams(location.search);createRoot(document.getElementById('root')).render(q.get('view')==='settings'?<Settings/>:<Dashboard/>);`);
fs.writeFileSync(path.join(out,'index.html'),'<!doctype html><html lang="fr"><meta charset="utf-8"><title>Offline Claude fixture</title><style>*{box-sizing:border-box}body{margin:0}</style><div id="root"></div><script type="module" src="./fixture.tsx"></script></html>');
await build({configFile:false,root:out,plugins:[react(),{name:'fixture-ipc',enforce:'pre',resolveId(id){if(/^@tauri-apps\/api\/(core|event)$/.test(id))return '\0fixture-ipc'},load(id){if(id==='\0fixture-ipc')return mock}}],build:{outDir:path.join(out,'site'),emptyOutDir:true}});
console.log('RENDERER_FIXTURE_BUILT '+out);
