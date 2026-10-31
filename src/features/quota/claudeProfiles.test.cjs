const fs = require('node:fs');
const Module = require('node:module');
const path = require('node:path');
const assert = require('node:assert/strict');
const { test } = require('node:test');
const ts = require('typescript');
const React = require('react');
const { renderToStaticMarkup } = require('react-dom/server');
let hooks = null;
let invokeCalls = [];
let invokeResult = null;
let invokeFailure = null;
let stateChanges = [];
let effects = [];
const load = Module._load;
Module._load = function (name, ...args) {
  if (name === 'react' && hooks) return { ...React, ...hooks };
  if (name === '@tauri-apps/api/core') return { invoke: async (command, payload) => {
    invokeCalls.push({ command, payload });
    if (invokeFailure) throw invokeFailure;
    return invokeResult;
  } };
  if (name === '@tauri-apps/api/event') return { listen: async () => () => {} };
  return load.call(this, name, ...args);
};
for (const ext of ['.ts', '.tsx']) require.extensions[ext] = (module, filename) => {
  module._compile(ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true, target: ts.ScriptTarget.ES2022 },
    fileName: filename,
  }).outputText, filename);
};
require.extensions['.scss'] = () => {};
function component(file, props, states = []) {
  let cursor = 0;
  effects = [];
  hooks = {
    useState: initial => {
      const index = cursor++;
      return [index < states.length ? states[index] : typeof initial === 'function' ? initial() : initial,
        value => stateChanges.push({ index, value })];
    },
    useRef: initial => ({ current: initial }), useId: () => 'fixture-details', useEffect: effect => effects.push(effect), useLayoutEffect: () => {}, useCallback: fn => fn,
  };
  const filename = require.resolve(file);
  for (const cached of Object.keys(require.cache)) {
    if (cached.startsWith(path.resolve(__dirname, '../..') + path.sep) && !cached.endsWith('.test.cjs')) delete require.cache[cached];
  }
  try { return require(file).default(props); } finally { hooks = null; }
}
const rows = ['five_hour', 'seven_day', 'seven_day_fable', 'extra_usage'].map((id, i) => ({ id, label: ['5 heures', '7 jours', 'Fable', 'Extra'][i], remaining: 80 - i * 10, exhausted: false, missing: '', resetText: 'lun. 2 nov. 12:00' }));
test('multi-account compact card shows adjacent 5h and 7d bars with remaining text and reset tooltips', () => {
  const tree = component('./ProviderCard.tsx', { name: 'Claude · Travail', compact: true, accountCompact: true, rows });
  const html = renderToStaticMarkup(tree);
  const bars = html.slice(html.indexOf('provider-card__paired-bars'), html.indexOf('</div></div>', html.indexOf('provider-card__paired-bars')));
  assert.equal((bars.match(/role="progressbar"/g) || []).length, 2);
  // v3.1.0 port: other limits (Fable, extra) stay visible below the pair.
  assert.equal((html.match(/role="progressbar"/g) || []).length, 4);
  assert.match(html, /provider-card__paired-bars/);
  assert.match(html, /80 %/);
  assert.match(html, /70 %/);
  assert.match(html, /title="[^"]*lun\. 2 nov/);
});
test('compact countdown follows each structured reset without changing quota percentages', () => {
  const now = Date.parse('2026-10-31T10:00:00Z');
  const current = [
    { ...rows[0], resetAt: new Date(now + 210 * 60000).toISOString() },
    { ...rows[1], resetAt: new Date(now + 80 * 3600000).toISOString() },
  ];
  const render = (time, data = current) => renderToStaticMarkup(component('./ProviderCard.tsx', { name: 'Claude · Travail', compact: true, accountCompact: true, rows: data, now: time }));
  assert.match(render(now), /aria-label="5 heures · temps restant : 3h30"/);
  assert.match(render(now), /3h30/);
  assert.match(render(now), /3j 8h/);
  assert.match(render(now + 60000), /3h29/);
  assert.match(render(now + 60000), /80 %/);
  assert.match(render(now, [{ ...current[0], resetAt: new Date(now + 42 * 60000).toISOString() }]), /42 min/);
  for (const resetAt of [undefined, null, 'invalid', new Date(now - 1).toISOString()]) {
    const html = render(now, [{ ...current[0], resetAt, remaining: 0, exhausted: true }]);
    assert.match(html, /—/);
    assert.match(html, /Épuisé/);
    assert.doesNotMatch(html, /3h30|NaN|-1 min/);
  }
});
const snapshot = { provider: 'claude', schemaVersion: 1, enabled: true, configured: true, state: 'ready', source: null, observedAt: null, providerFetchedAt: null, message: '', errorKind: null, metrics: [], capability: { refresh: 'on_demand' }, planType: 'Pro', extraUsage: null };
const account = (id, primary = false) => ({ id, path: '/fixtures/' + id, name: id, primary, visible: true, available: true, refreshing: false, snapshot });
const accounts = [account('Principal', true), account('Travail')];
const prefs = { codexEnabled: false, claudeEnabled: true, indicator: 'claude', ntfy: { enabled: false, serverUrl: '', topic: '' }, claudeAccounts: { mode: 'single', selectedId: null, profiles: [] } };
test('dashboard owns one local minute tick, cleans it up and never fetches on that tick', () => {
  let tick, cleared;
  global.window = { setInterval: (fn, ms) => { assert.equal(ms, 60000); tick = fn; return 123; }, clearInterval: id => { cleared = id; } };
  invokeCalls = []; stateChanges = [];
  component('../../pages/Dashboard/Dashboard.tsx', {});
  // Run only the local-clock effect, not IPC effects.
  const clock = effects.find(effect => String(effect).includes('setInterval'));
  assert.ok(clock, 'a local minute clock exists');
  const cleanup = clock();
  tick();
  assert.equal(stateChanges.length, 1);
  assert.equal(typeof stateChanges[0].value, 'number');
  assert.deepEqual(invokeCalls, []);
  cleanup();
  assert.equal(cleared, 123);
  delete global.window;
});
test('settings discovers profiles, offers Unique/Multi and selects principal without switching sessions', () => {
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, false, false, accounts]);
  const html = renderToStaticMarkup(tree);
  assert.match(html, /Unique/);
  assert.match(html, /Multi/);
  assert.match(html, /Compte Claude affiché/);
  assert.match(html, /Travail/);
  assert.match(html, /Chemin absolu/);
});
test('dashboard multi mode shows every visible profile, independent of selected account', () => {
  const tree = component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi', selectedId: 'Travail' } }, false, null, null, null, false, 0, accounts]);
  const html = renderToStaticMarkup(tree);
  assert.match(html, /Claude · Principal/);
  assert.match(html, /Claude · Travail/);
});
test('multi-account 5h bar inherits Unique thickness with a thinner weekly bar and 2px gap', () => {
  const sass = require('sass');
  const css = sass.compile(path.join(__dirname, '../../pages/Dashboard/dashboard.scss')).css;
  assert.match(css, /\.provider-card__paired-bars\s*\{[^}]*gap: 2px/s);
  assert.match(css, /\.dashboard__progress\s*\{[^}]*height: 7px/s);
  assert.doesNotMatch(css, /\.provider-card__window--short\s*\{[^}]*height:/s);
  assert.match(css, /\.provider-card__window--week\s*\{[^}]*height: 3px/s);
});
test('multi-profile disclosure preserves every metric even when Fable is reported first', () => {
  const tree = component('./ProviderCard.tsx', { name: 'Claude · Travail', compact: true, accountCompact: true, rows: [rows[2], rows[0], rows[1], rows[3]] }, [false, true, false]);
  const html = renderToStaticMarkup(tree);
  assert.match(html, /aria-label="Fable"/);
  assert.match(html, /aria-label="Extra"/);
  assert.match(html, /60 %/);
});
function find(tree, predicate) {
  if (!tree || typeof tree !== 'object') return null;
  if (predicate(tree)) return tree;
  for (const child of React.Children.toArray(tree.props?.children)) {
    const found = find(child, predicate);
    if (found) return found;
  }
  return null;
}
function settingsProps(overrides = {}) {
  return { prefs, accounts, busy: false, onUpdate: async () => true, onAdd: async () => true, ...overrides };
}
test('single account keeps existing presentation and hides mode/select controls', () => {
  const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ accounts: [accounts[0]] }));
  const html = renderToStaticMarkup(tree);
  assert.doesNotMatch(html, /claude-mode|Compte Claude affiché|role="switch"/);
  assert.match(html, /Principal/);
});
test('a sole remaining profile can be unhidden without showing the segmented control', () => {
  const current = { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi', profiles: [{ id: 'Principal', path: '/fixtures/Principal', name: 'Principal', visible: false }] } };
  const html = renderToStaticMarkup(component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ prefs: current, accounts: [{ ...accounts[0], visible: false }] })));
  assert.match(html, /aria-label="Afficher Principal"/);
  assert.doesNotMatch(html, /claude-mode/);
});
test('single selection only changes selectedId and preserves visibility and provider preferences', () => {
  let patch;
  const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ onUpdate: async value => { patch = value; return true; } }));
  find(tree, node => node.type === 'select').props.onChange({ target: { value: 'Travail' } });
  assert.deepEqual(patch, { claudeAccounts: { ...prefs.claudeAccounts, selectedId: 'Travail' } });
});
test('multi visibility edit merges discovered metadata and preserves other profiles', () => {
  let patch;
  const stored = { id: 'Other', path: '/fixtures/Other', name: 'Other', visible: true };
  const current = { ...prefs, claudeAccounts: { mode: 'multi', selectedId: 'Travail', profiles: [stored] } };
  const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ prefs: current, onUpdate: async value => { patch = value; return true; } }));
  find(tree, node => node.props?.['aria-label'] === 'Afficher Travail').props.onChange({ target: { checked: false } });
  assert.deepEqual(patch.claudeAccounts, { mode: 'multi', selectedId: 'Travail', profiles: [stored, { id: 'Travail', path: '/fixtures/Travail', name: 'Travail', visible: false }] });
  assert.equal(patch.claudeEnabled, undefined);
});
test('renaming while provider is disabled preserves stored visibility', async () => {
  let patch;
  const profile = { id: 'Travail', path: '/fixtures/Travail', name: 'Travail', visible: false };
  const current = { ...prefs, claudeEnabled: false, claudeAccounts: { ...prefs.claudeAccounts, profiles: [profile] } };
  const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ prefs: current, accounts: [{ ...accounts[1], visible: false }], onUpdate: async value => { patch = value; return true; } }));
  const rename = find(tree, node => node.type?.name === 'ProfileName');
  await rename.props.onSave('Professionnel');
  assert.deepEqual(patch.claudeAccounts.profiles, [{ ...profile, name: 'Professionnel' }]);
});
test('renaming a discovered profile does not copy single-view visibility into multi preferences', async () => {
  let patch;
  const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ accounts: [accounts[1]], onUpdate: async value => { patch = value; return true; } }));
  const rename = find(tree, node => node.type?.name === 'ProfileName');
  await rename.props.onSave('Secondaire');
  assert.equal(patch.claudeAccounts.profiles[0].visible, false);
});
test('manual add rejects relative paths and forwards absolute paths literally', async () => {
  const added = [];
  for (const value of ['relative/profile', '/fixtures/Profile with spaces']) {
    const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ onAdd: async value => { added.push(value); return true; } }), [value, null]);
    await find(tree, node => node.props?.className === 'claude-accounts__add').props.onSubmit({ preventDefault() {} });
  }
  assert.deepEqual(added, ['/fixtures/Profile with spaces']);
});
test('settings saves full preferences and accepts canonical add result without rebuilding metadata', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = null;
  const canonical = { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, profiles: [{ id: 'canonical', path: '/canonical/path', name: 'Added', visible: true }] } };
  invokeResult = canonical;
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, false, false, accounts]);
  const controls = find(tree, node => node.type?.name === 'ClaudeAccountsSettings');
  assert.equal(await controls.props.onUpdate({ claudeAccounts: { ...prefs.claudeAccounts, selectedId: 'Travail' } }), true);
  assert.deepEqual(invokeCalls[0], { command: 'set_preferences', payload: { prefs: { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, selectedId: 'Travail' } } } });
  assert.equal(await controls.props.onAdd('/fixtures/link'), true);
  assert.deepEqual(invokeCalls[1], { command: 'add_claude_profile', payload: { path: '/fixtures/link' } });
  assert.ok(stateChanges.some(change => change.index === 1 && change.value === canonical));
});
test('settings exposes failed manual add and does not report success', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = new Error('Dossier introuvable');
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, false, false, accounts]);
  const controls = find(tree, node => node.type?.name === 'ClaudeAccountsSettings');
  assert.equal(await controls.props.onAdd('/missing'), false);
  assert.ok(stateChanges.some(change => String(change.value).includes('Dossier introuvable')));
  invokeFailure = null;
});
test('dashboard single mode uses selected profile even if hidden in multi mode', () => {
  const metric = { id: 'claude:five_hour', label: 'Sélection', usedPercent: 19, resetAt: null, availability: 'available', periodDurationSeconds: 18000, exhausted: false };
  const tree = component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, selectedId: 'Travail' } }, false, null, null, null, false, 0, [accounts[0], { ...accounts[1], visible: false, snapshot: { ...snapshot, metrics: [metric] } }]]);
  const html = renderToStaticMarkup(tree);
  assert.match(html, /Sélection/);
  assert.match(html, /81 %/);
  assert.doesNotMatch(html, /Claude · Travail|paired-bars/);
});
test('one visible account keeps the exact v3.1.0 card, without an added extra-usage note', () => {
  const extra = { ...accounts[0], snapshot: { ...snapshot, extraUsage: { enabled: true, limitReached: true } } };
  const html = renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, prefs, false, null, null, null, false, 0, [extra]]));
  assert.doesNotMatch(html, /Extra · plafond atteint|Usage supplémentaire/);
  assert.doesNotMatch(html, /paired-bars/);
});
test('multi account card still reports paid extra usage per account', () => {
  const extra = { ...accounts[1], snapshot: { ...snapshot, extraUsage: { enabled: true, limitReached: true } } };
  const html = renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi' } }, false, null, null, null, false, 0, [accounts[0], extra]]));
  assert.match(html, /Extra · plafond atteint/);
});
const fableRow = { id: 'claude:weekly_fable', label: 'Limite Fable', modelLabel: 'Fable', remaining: 60, exhausted: false, missing: '', resetText: 'mar. 3 nov. 12:00' };
test('multi card keeps a separate Fable row visible with its model label (nothing hidden to fit)', () => {
  const now = Date.parse('2026-10-31T10:00:00Z');
  const html = renderToStaticMarkup(component('./ProviderCard.tsx', { name: 'Claude · Travail', compact: true, accountCompact: true, now,
    rows: [{ ...rows[0], id: 'claude:five_hour' }, { ...rows[1], id: 'claude:seven_day' }, fableRow] }));
  assert.equal((html.match(/role="progressbar"/g) || []).length, 3);
  assert.match(html, /provider-row__model-label">Fable/);
  assert.match(html, /60 %/);
});
test('coalesced weekly Fable is the 7 j bar and keeps the Fable name visible', () => {
  const now = Date.parse('2026-10-31T10:00:00Z');
  const html = renderToStaticMarkup(component('./ProviderCard.tsx', { name: 'Claude · Travail', compact: true, accountCompact: true, now,
    rows: [{ ...rows[0], id: 'claude:five_hour' }, { ...fableRow, resetAt: new Date(now + 80 * 3600000).toISOString() }] }));
  assert.equal((html.match(/role="progressbar"/g) || []).length, 2);
  assert.match(html, /provider-card__window--week/);
  assert.match(html, /Fable · 3j 8h/);
});
test('Fable preference applies to every multi account card', () => {
  const fable = { id: 'claude:weekly_fable', label: 'Limite hebdomadaire Fable', usedPercent: 40, resetAt: null, availability: 'available', periodDurationSeconds: 604800, exhausted: false };
  const withFable = (a) => ({ ...a, snapshot: { ...snapshot, metrics: [fable] } });
  const current = [withFable(accounts[0]), withFable(accounts[1])];
  const render = (fableEnabled) => renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, fableEnabled, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi' } }, false, null, null, null, false, 0, current]));
  assert.equal((render(true).match(/Fable/g) || []).length >= 2, true);
  assert.doesNotMatch(render(false), /Fable/);
});
test('multi mode with one visible account keeps expanded legacy layout', () => {
  const metric = { id: 'claude:five_hour', label: '5 h', usedPercent: 19, resetAt: null, availability: 'available', periodDurationSeconds: 18000, exhausted: false };
  const current = [{ ...accounts[0], snapshot: { ...snapshot, metrics: [metric] } }, { ...accounts[1], visible: false }];
  const html = renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi' } }, false, null, null, null, false, 0, current]));
  assert.doesNotMatch(html, /paired-bars|Claude · Principal/);
  assert.match(html, /81 %/);
});
test('dashboard hides only invisible multi profiles and never shows disabled Claude', () => {
  for (const enabled of [true, false]) {
    const tree = component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, claudeEnabled: enabled, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi' } }, false, null, null, null, false, 0, [accounts[0], { ...accounts[1], visible: false }]]);
    const html = renderToStaticMarkup(tree);
    assert.doesNotMatch(html, /Claude · Travail/);
    assert.equal(html.includes('Quotas Claude'), enabled);
  }
});
test('Codex compact and expanded cards retain their existing row counts', () => {
  for (const compact of [true, false]) {
    const html = renderToStaticMarkup(component('./ProviderCard.tsx', { name: 'Codex', rows, compact }));
    assert.equal((html.match(/role="progressbar"/g) || []).length, compact ? 1 : 4);
    assert.doesNotMatch(html, /paired-bars/);
  }
});
test('multi accounts stack on narrow windows without changing legacy two-provider grids', () => {
  const css = require('sass').compile(path.join(__dirname, '../../pages/Dashboard/dashboard.scss')).css;
  assert.match(css, /@media\s*\(max-width: 520px\)\s*\{\s*\.provider-grid--accounts\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\)/s);
});
test('settings discovery remains available with provider off and reports discovery failure', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = new Error('Discovery fixture error');
  global.window = { setTimeout: () => 1, clearTimeout() {} };
  component('../settings/Settings.tsx', {}, ['providers', { ...prefs, claudeEnabled: false }, null, null, null, false, 0, false, false, []]);
  const cleanup = effects.find(effect => String(effect).includes('get_claude_accounts'))();
  await new Promise(resolve => setImmediate(resolve));
  cleanup();
  invokeFailure = null;
  assert.ok(invokeCalls.some(call => call.command === 'get_claude_accounts'));
  assert.ok(stateChanges.some(change => String(change.value).includes('découvrir les comptes Claude')));
  delete global.window;
});
