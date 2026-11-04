// Offscreen proof that the standalone window follows its content: the REAL CompactWindow runs in
// headless Chrome with a mocked Tauri bridge. `fit_main_window` is simulated by resizing the
// viewport to the requested height (no title-bar inset, no AppKit). Not a native window test.
//   node scripts/window-fit-check.mjs   (MODELS=<dir of *.json>; default evidence/t_e0f6e24a/native)
import { build } from 'vite';
import react from '@vitejs/plugin-react';
import { spawn } from 'node:child_process';
import fs from 'node:fs'; import path from 'node:path'; import http from 'node:http'; import assert from 'node:assert/strict';

const out = path.resolve(process.env.SHOTS || 'node_modules/.cache/window-fit/shots');
const models = path.resolve(process.env.MODELS || 'evidence/t_e0f6e24a/native');
const dir = path.resolve('node_modules/.cache/window-fit');
fs.mkdirSync(out, { recursive: true }); fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(path.join(dir, 'core.ts'), `export async function invoke(command: string, args?: any) {
  const w = window as any; (w.__calls ||= []).push([command, args]);
  if (command === 'get_compact_panel') return w.__model;
  if (command === 'fit_main_window') { (w.__fits ||= []).push(args); }
  return null;
}`);
fs.writeFileSync(path.join(dir, 'event.ts'), `export async function listen() { return () => {}; }`);
fs.writeFileSync(path.join(dir, 'fixture.tsx'), `import React from 'react';import { createRoot } from 'react-dom/client';
import CompactWindow from ${JSON.stringify(path.resolve('src/features/compact/CompactWindow.tsx'))};
(window as any).__QUOTA_MONITOR_VIEW__ = 'compact';
createRoot(document.getElementById('root')!).render(<CompactWindow />);`);
fs.writeFileSync(path.join(dir, 'index.html'), '<!doctype html><html lang="fr"><meta charset="utf-8"><style>*{box-sizing:border-box}body{margin:0}</style><div id="root"></div><script type="module" src="./fixture.tsx"></script>');
await build({
  configFile: false, root: dir, logLevel: 'warn', plugins: [react()],
  resolve: { alias: { '@tauri-apps/api/core': path.join(dir, 'core.ts'), '@tauri-apps/api/event': path.join(dir, 'event.ts') } },
  build: { outDir: path.join(dir, 'site'), emptyOutDir: true },
});
const site = path.join(dir, 'site');
const server = http.createServer((req, res) => {
  const u = new URL(req.url, 'http://x').pathname; const f = path.join(site, u.endsWith('/') ? u + 'index.html' : u);
  if (!f.startsWith(site) || !fs.existsSync(f)) { res.writeHead(404); res.end(); return; }
  res.setHeader('Content-Type', { js: 'text/javascript', css: 'text/css', html: 'text/html' }[f.split('.').pop()] || 'application/octet-stream');
  res.end(fs.readFileSync(f));
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const shell = process.env.CHROME_SHELL || process.env.HOME + '/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome = spawn(shell, ['--remote-debugging-port=9398', '--user-data-dir=' + path.join(out, '.profile'), '--no-first-run', 'about:blank'], { stdio: 'ignore' });
const kill = setTimeout(() => { chrome.kill('SIGKILL'); process.exit(2); }, 280000);
const sleep = ms => new Promise(r => setTimeout(r, ms));
let ws; const errors = []; const report = [];
try {
  let target; for (let i = 0; i < 50 && !target; i++) { await sleep(200); try { target = (await (await fetch('http://127.0.0.1:9398/json')).json()).find(t => t.type === 'page'); } catch {} }
  assert.ok(target, 'browser started');
  ws = new WebSocket(target.webSocketDebuggerUrl); await new Promise(r => ws.addEventListener('open', r, { once: true }));
  let id = 0; const pending = new Map();
  ws.addEventListener('message', e => { const m = JSON.parse(e.data); if (m.id) { pending.get(m.id)?.(m); pending.delete(m.id); } else if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.text); });
  const cdp = (method, params = {}) => new Promise((res, rej) => { const n = ++id; pending.set(n, m => m.error ? rej(Error(m.error.message)) : res(m.result)); ws.send(JSON.stringify({ id: n, method, params })); });
  const ev = async e => (await cdp('Runtime.evaluate', { expression: e, returnByValue: true, awaitPromise: true })).result.value;
  await cdp('Page.enable'); await cdp('Runtime.enable');
  const WIDTH = 360;
  const viewport = h => cdp('Emulation.setDeviceMetricsOverride', { width: WIDTH, height: Math.round(h), deviceScaleFactor: 2, mobile: false });
  const sequence = ['single-account', 'single-account-classic', 'single-account', 'multi-accounts', 'multi-accounts-classic', 'multi-fable', 'multi-fable-classic', 'multi-fable', 'multi-accounts'];
  const content = () => ev(`(() => { const n = document.querySelector('.compact-fit'); return { content: n.getBoundingClientRect().height, top: n.getBoundingClientRect().top, inner: innerHeight, scroll: document.documentElement.scrollHeight, fits: (window.__fits || []).length }; })()`);
  for (const theme of ['light', 'dark']) {
    await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }, { name: 'prefers-reduced-motion', value: 'reduce' }] });
    await viewport(520); // initial window height (520) like show_main_window
    const first = JSON.parse(fs.readFileSync(path.join(models, sequence[0] + '.json'), 'utf8'));
    await cdp('Page.navigate', { url: `http://127.0.0.1:${port}/` });
    await sleep(300);
    await ev(`window.__model = ${JSON.stringify(first)}; 0`);
    await cdp('Page.reload'); // pick the model on first poll
    await ev(`window.__model = ${JSON.stringify(first)}; 0`);
    for (let step = 0; step < sequence.length; step++) {
      const name = sequence[step];
      const model = JSON.parse(fs.readFileSync(path.join(models, name + '.json'), 'utf8'));
      const before = await content();
      await ev(`window.__model = ${JSON.stringify(model)}; 0`);
      // Wait for the 2 s poll, the observer and the 40 ms debounce.
      await sleep(2700);
      const request = await ev(`(window.__fits || []).at(-1)`);
      assert.ok(request, `${theme}/${name}: no fit request`);
      await viewport(request.height); // the "native" resize, top edge unchanged by construction
      await sleep(400);
      const after = await content();
      assert.ok(Math.abs(after.inner - after.content) <= 1, `${theme}/${name}: window ${after.inner} vs content ${after.content}`);
      assert.ok(after.scroll <= after.inner + 1, `${theme}/${name}: unnecessary scroll ${after.scroll} > ${after.inner}`);
      assert.equal(after.top, 0, 'top edge stable');
      await sleep(600);
      assert.equal((await content()).fits, after.fits, `${theme}/${name}: resize loop`);
      const row = { theme, name, window: after.inner, content: after.content, scroll: after.scroll, fits: after.fits };
      report.push(row);
      fs.writeFileSync(path.join(out, `${theme}-${String(step).padStart(2, '0')}-${name}.png`), Buffer.from((await cdp('Page.captureScreenshot', { format: 'png' })).data, 'base64'));
    }
  }
  assert.deepEqual(errors, []);
  // The height must actually differ across presentations (otherwise the check proves nothing).
  const heights = new Map(report.filter(r => r.theme === 'light').map(r => [r.name, r.window]));
  assert.ok(heights.get('single-account-classic') > heights.get('single-account'), JSON.stringify([...heights]));
  assert.ok(heights.get('multi-fable') > heights.get('multi-accounts'), JSON.stringify([...heights]));
  fs.writeFileSync(path.join(out, 'report.json'), JSON.stringify(report, null, 2));
  console.log(`WINDOW_FIT_PASS ${report.length} steps; window = content ±1 px, no scroll, top 0, no loop`);
  console.table(report.filter(r => r.theme === 'light'));
} finally { clearTimeout(kill); ws?.close(); chrome.kill(); server.close(); }
