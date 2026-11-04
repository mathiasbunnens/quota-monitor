// Offline Fable geometry across real dashboard and standalone compact renderers.
// First run claude-renderer-fixture.mjs and compact-accounts-check.mjs with native smoke models.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';
const out = path.resolve(process.env.SHOTS || 'node_modules/.cache/t-d5-fable/geometry');
fs.mkdirSync(out, { recursive: true });
const roots = {
  dashboard: path.resolve(process.env.DASHBOARD_SITE || 'node_modules/.cache/t-d5-fable/dashboard/site'),
  compact: path.resolve(process.env.COMPACT_SITE || 'node_modules/.cache/qm-compact-after/site'),
};
const servers = {};
const ports = {};
for (const [name, root] of Object.entries(roots)) {
  const server = http.createServer((req, res) => {
    const pathname = new URL(req.url, 'http://fixture').pathname;
    const file = path.resolve(root, '.' + (pathname.endsWith('/') ? pathname + 'index.html' : pathname));
    if (!file.startsWith(root + path.sep) || !fs.existsSync(file)) { res.writeHead(404); res.end(); return; }
    res.setHeader('Content-Type', ({ js: 'text/javascript', css: 'text/css', html: 'text/html', json: 'application/json' })[file.split('.').pop()] || 'application/octet-stream');
    res.end(fs.readFileSync(file));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  servers[name] = server; ports[name] = server.address().port;
}
const shell = process.env.CHROME_SHELL || process.env.HOME + '/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell';
const chrome = spawn(shell, ['--remote-debugging-port=9397', '--user-data-dir=' + path.join(out, '.profile'), '--no-first-run', '--hide-scrollbars', 'about:blank'], { stdio: 'ignore' });
const timer = setTimeout(() => { chrome.kill('SIGKILL'); process.exit(2); }, 180000);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let ws;
const report = [];
try {
  let target;
  for (let i = 0; i < 50 && !target; i++) { await sleep(200); try { target = (await (await fetch('http://127.0.0.1:9397/json')).json()).find(t => t.type === 'page'); } catch {} }
  assert.ok(target, 'headless browser started');
  ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise(resolve => ws.addEventListener('open', resolve, { once: true }));
  let id = 0; const pending = new Map(); const errors = [];
  ws.addEventListener('message', e => {
    const message = JSON.parse(e.data);
    if (message.id) { pending.get(message.id)?.(message); pending.delete(message.id); }
    else if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails.text);
  });
  const cdp = (method, params = {}) => new Promise((resolve, reject) => {
    const n = ++id; pending.set(n, m => m.error ? reject(Error(m.error.message)) : resolve(m.result));
    ws.send(JSON.stringify({ id: n, method, params }));
  });
  const ev = async expression => (await cdp('Runtime.evaluate', { expression, returnByValue: true })).result.value;
  await cdp('Page.enable'); await cdp('Runtime.enable');
  for (const renderer of Object.keys(roots)) for (const theme of ['light', 'dark']) for (const multi of [false, true]) for (const scene of ['absent', 'fable', 'same-reset', 'classic']) {
    const fable = scene !== 'absent'; const classic = scene === 'classic';
    const model = fable ? (multi ? 'multi-fable' : 'single-fable') : (multi ? 'multi-accounts' : 'single-account');
    // Classic mono uses the same real three-row model with only its layout flag changed.
    if (classic && !multi) {
      const source = JSON.parse(fs.readFileSync(path.join(roots.compact, 'models/single-fable.json')));
      source.providers.forEach(p => { p.compact = false; });
      fs.writeFileSync(path.join(roots.compact, 'models/single-fable-classic.json'), JSON.stringify(source));
    }
    if (scene === 'same-reset') {
      const source = JSON.parse(fs.readFileSync(path.join(roots.compact, `models/${model}.json`)));
      for (const provider of source.providers) {
        const fable = provider.rows.find(r => r.model);
        const week = provider.rows.filter(r => !r.model)[1];
        if (fable && week) fable.reset = week.reset;
      }
      fs.writeFileSync(path.join(roots.compact, `models/${model}-same.json`), JSON.stringify(source));
    }
    const query = renderer === 'compact' ? `model=${model}${classic ? '-classic' : scene === 'same-reset' ? '-same' : ''}`
      : `scenario=${fable ? 'fable' : 'multi'}&fableReset=${scene === 'same-reset' ? 'same' : 'different'}&only=${multi ? '2' : '1'}&layout=${classic ? 'classic' : 'compact'}&clock=controlled&platform=macos`;
    await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }, { name: 'prefers-reduced-motion', value: 'reduce' }] });
    await cdp('Emulation.setDeviceMetricsOverride', { width: renderer === 'compact' ? 324 : 520, height: 900, deviceScaleFactor: 2, mobile: false });
    await cdp('Page.navigate', { url: `http://127.0.0.1:${ports[renderer]}/?${query}` });
    await sleep(650);
    const compact = renderer === 'compact';
    const result = await ev(`(() => {
      const compact = ${compact};
      const root = compact ? document : document.querySelector('#claude-group').closest('section');
      const rect = el => { const r = el.getBoundingClientRect(); return { y:r.y, bottom:r.bottom, height:r.height }; };
      const cards = [...root.querySelectorAll(compact ? '.compact-card' : '.provider-card')];
      return { overflow:document.documentElement.scrollWidth > innerWidth, cards:cards.map(card => {
        const pair = card.querySelector(compact ? '.compact-pair' : '.provider-card__paired-bars');
        const tracks = [...card.querySelectorAll(compact ? '[role=meter]' : '[role=progressbar]')];
        const caption = card.querySelector(compact ? '.compact-pair__fable-caption' : '.provider-card__fable-caption');
        return { tracks:tracks.map(rect), fills:tracks.map(t=>t.firstElementChild?rect(t.firstElementChild):null), pairHeight:pair?rect(pair).height:null,
          caption:caption?.innerText, clipped:caption?[...caption.children].some(s=>s.scrollWidth>s.clientWidth+1):false,
          visible:card.innerText, details:tracks.map(t=>({title:t.title,label:t.getAttribute('aria-label')})),
          resets:card.querySelectorAll(compact ? '.compact-row__reset' : '.provider-row__reset').length };
      }) };
    })()`);
    assert.ok(result, `${renderer} loaded`);
    assert.equal(result.overflow, false);
    assert.equal(result.cards.length, multi ? 2 : 1);
    for (const card of result.cards) {
      assert.equal(card.tracks.length, fable ? 3 : 2);
      assert.equal(card.clipped, false, JSON.stringify(card));
      const heights = card.tracks.map(t => t.height);
      if (classic) {
        assert.equal(card.pairHeight, null); assert.equal(card.resets, 3);
        assert.deepEqual(heights, Array(3).fill(compact ? 5 : 7));
      } else {
        assert.deepEqual(heights, fable ? [compact ? 5 : 7, 3, 3] : [compact ? 5 : 7, 3]);
        assert.equal(card.resets, 0);
        if (compact) assert.equal(card.pairHeight, fable ? 65 : 60);
        assert.equal(card.caption, undefined);
        assert.doesNotMatch(card.visible, /Fable/);
        if (fable) {
          assert.equal(card.tracks[2].y - card.tracks[1].bottom, 2);
          assert.equal(card.fills[2]?.height, 3);
          for (const text of Object.values(card.details[2])) assert.match(text, /Fable.*%.*[Rr]éinitialisation/);
        } else assert.equal(card.caption, undefined);
      }
    }
    const name = `${renderer}-${multi ? 'multi' : 'mono'}-${scene}-${theme}`;
    fs.writeFileSync(path.join(out, name + '.png'), Buffer.from((await cdp('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true })).data, 'base64'));
    report.push({ name, ...result });
  }
  assert.deepEqual(errors, []);
  assert.equal(report.length, 32);
  fs.writeFileSync(path.join(out, 'report.json'), JSON.stringify(report, null, 2));
  console.log(`FABLE_GEOMETRY_PASS ${report.length} scenes: light/dark mono/multi absent/present/classic; track=fill=3 gap=2; no clipping`);
} finally {
  clearTimeout(timer); ws?.close(); chrome.kill(); Object.values(servers).forEach(server => server.close());
}
