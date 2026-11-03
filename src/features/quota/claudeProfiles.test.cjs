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
function component(file, props, states = [], exportName = 'default') {
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
  try { return require(file)[exportName](props); } finally { hooks = null; }
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
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, accounts]);
  const html = renderToStaticMarkup(tree);
  assert.match(html, /name="claude-mode"[^>]*\/>Unique/);
  assert.match(html, /Multi/);
  // Unique: one flat list, the displayed account chosen inline (radio), no
  // duplicated summary, select or nested disclosure.
  assert.match(html, /<input(?=[^>]*name="claude-selected")(?=[^>]*aria-label="Afficher Principal")(?=[^>]*checked="")/);
  assert.match(html, /aria-label="Afficher Travail"/);
  assert.doesNotMatch(html, /<details|<select|Compte Claude affiché|Gérer les comptes/);
  assert.equal((html.match(/class="claude-accounts__label">Travail</g) || []).length, 1);
  assert.match(html, />Ajouter un compte<\/button>/);
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
  find(tree, node => node.props?.['aria-label'] === 'Afficher Travail').props.onChange();
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
  const rename = find(tree, node => node.type?.name === 'AccountName');
  await rename.props.onSave('Professionnel');
  assert.deepEqual(patch.claudeAccounts.profiles, [{ ...profile, name: 'Professionnel' }]);
});
test('renaming a discovered profile does not copy single-view visibility into multi preferences', async () => {
  let patch;
  const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ accounts: [accounts[1]], onUpdate: async value => { patch = value; return true; } }));
  const rename = find(tree, node => node.type?.name === 'AccountName');
  await rename.props.onSave('Secondaire');
  assert.equal(patch.claudeAccounts.profiles[0].visible, false);
});
test('manual add rejects relative paths and forwards absolute paths literally', async () => {
  const added = [];
  for (const value of ['relative/profile', '/fixtures/Profile with spaces']) {
    const tree = component('../settings/ClaudeAccountsSettings.tsx', settingsProps({ onAdd: async value => { added.push(value); return true; } }), [value, null, true]);
    await find(tree, node => node.props?.className === 'claude-accounts__manual').props.onSubmit({ preventDefault() {} });
    await new Promise(resolve => setImmediate(resolve));
  }
  assert.deepEqual(added, ['/fixtures/Profile with spaces']);
});
test('settings saves full preferences and accepts canonical add result without rebuilding metadata', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = null;
  const canonical = { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, profiles: [{ id: 'canonical', path: '/canonical/path', name: 'Added', visible: true }] } };
  invokeResult = canonical;
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, accounts]);
  const controls = find(tree, node => node.type?.name === 'ClaudeAccountsSettings');
  assert.equal(await controls.props.onUpdate({ claudeAccounts: { ...prefs.claudeAccounts, selectedId: 'Travail' } }), true);
  assert.deepEqual(invokeCalls[0], { command: 'set_preferences', payload: { prefs: { ...prefs, claudeAccounts: { ...prefs.claudeAccounts, selectedId: 'Travail' } } } });
  assert.equal(await controls.props.onAdd('/fixtures/link'), true);
  assert.deepEqual(invokeCalls[1], { command: 'add_claude_profile', payload: { path: '/fixtures/link' } });
  assert.ok(stateChanges.some(change => change.index === 1 && change.value === canonical));
});
test('settings exposes failed manual add and does not report success', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = new Error('Dossier introuvable');
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, accounts]);
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
test('a sole Compacte card pairs its bars without an added name or extra-usage note', () => {
  const extra = { ...accounts[0], snapshot: { ...snapshot, extraUsage: { enabled: true, limitReached: true } } };
  for (const layout of ['compact', 'classic']) {
    const html = renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, layout }, false, null, null, null, false, 0, [extra]]));
    assert.doesNotMatch(html, /Extra · plafond atteint|Usage supplémentaire|Claude · Principal/);
    assert.equal(/paired-bars/.test(html), layout === 'compact');
  }
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
test('multi mode with one visible account follows the saved layout (Classique = expanded rows)', () => {
  const metric = { id: 'claude:five_hour', label: '5 h', usedPercent: 19, resetAt: null, availability: 'available', periodDurationSeconds: 18000, exhausted: false };
  const current = [{ ...accounts[0], snapshot: { ...snapshot, metrics: [metric] } }, { ...accounts[1], visible: false }];
  const render = (layout) => renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, [[], null, snapshot, { ...prefs, layout, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi' } }, false, null, null, null, false, 0, current]));
  assert.doesNotMatch(render('classic'), /paired-bars|Claude · Principal/);
  assert.match(render('classic'), /81 %/);
  assert.match(render('compact'), /paired-bars/);
  assert.doesNotMatch(render('compact'), /Claude · Principal/);
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
  component('../settings/Settings.tsx', {}, ['providers', { ...prefs, claudeEnabled: false }, null, null, null, false, 0, []]);
  const cleanup = effects.find(effect => String(effect).includes('get_claude_accounts'))();
  await new Promise(resolve => setImmediate(resolve));
  cleanup();
  invokeFailure = null;
  assert.ok(invokeCalls.some(call => call.command === 'get_claude_accounts'));
  assert.ok(stateChanges.some(change => String(change.value).includes('découvrir les comptes Claude')));
  delete global.window;
});

// ---- Fournisseurs → Claude : liste plate, ajout direct ---------------------
const accountsUx = (props, states = []) => component('../settings/ClaudeAccountsSettings.tsx', settingsProps(props), states);
const signedOut = { ...snapshot, state: 'error', errorKind: 'not_signed_in', planType: null, message: 'Profil non connecté : CLAUDE_CONFIG_DIR=/fixtures/x claude auth login.' };
const addButton = (tree) => find(tree, node => node.props?.className === 'claude-accounts__add');
test('zero or one profile: the Add action is there at once, no panel, prose or disclosure', () => {
  for (const list of [[], [accounts[0]]]) {
    const html = renderToStaticMarkup(accountsUx({ accounts: list }));
    assert.match(html, /class="claude-accounts__add"[^>]*>.*Ajouter un compte<\/button>/);
    assert.doesNotMatch(html, /claude-mode|<details|Suivre un autre|profil existant|Chemin absolu|role="switch"/);
    assert.doesNotMatch(html, /<p[ >]/);
  }
  // One profile: name, plan and state in one compact row, rename by pencil.
  const one = renderToStaticMarkup(accountsUx({ accounts: [accounts[0]] }));
  assert.match(one, /class="claude-accounts__label">Principal<\/strong><button[^>]*aria-label="Renommer Principal"/);
  assert.match(one, /<small>Principal<\/small><span class="prefs-plan">Pro<\/span>/);
  assert.match(one, /prefs-pill--good">Connecté/);
  assert.doesNotMatch(one, /aria-label="Nom du compte/);
});
test('« Ajouter » opens the folder chooser directly and forwards the chosen path literally', async () => {
  for (const [picked, expected] of [['/Users/fixture/.claude-pro/', ['/Users/fixture/.claude-pro/']], [null, []]]) {
    const added = [];
    const tree = accountsUx({ onChooseFolder: async () => picked, onAdd: async value => { added.push(value); return true; } });
    await addButton(tree).props.onClick();
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(added, expected);
  }
});
test('without a native chooser « Ajouter » opens the single secondary area with the path field', async () => {
  stateChanges = [];
  global.window = { requestAnimationFrame: () => 0 };
  const tree = accountsUx({});
  await addButton(tree).props.onClick();
  assert.ok(stateChanges.some(change => change.index === 2 && change.value === true));
  delete global.window;
  const open = renderToStaticMarkup(accountsUx({}, ['', null, true]));
  assert.match(open, /aria-label="Chemin absolu d’un profil Claude Code"/);
  assert.match(open, /<code>\/fixtures\/Travail<\/code>/);
  assert.doesNotMatch(open, /<details/);
});
test('add errors stay concise next to the action; busy disables add/rename, switches keep focus', () => {
  const html = renderToStaticMarkup(accountsUx({ actionError: '« Travail » est déjà affiché. Choisissez un autre profil Claude Code.', onChooseFolder: async () => null }));
  assert.match(html, /role="alert"[^>]*>« Travail » est déjà affiché/);
  const current = { ...prefs, claudeAccounts: { mode: 'multi', selectedId: null, profiles: [] } };
  const busy = renderToStaticMarkup(accountsUx({ prefs: current, busy: true, onChooseFolder: async () => null }, ['/x', null, true]));
  assert.match(busy, /aria-busy="true"/);
  assert.match(busy, /class="claude-accounts__add" disabled=""/);
  assert.match(busy, /disabled=""[^>]*aria-label="Renommer Travail"/);
  // Switches/radios stay enabled so keyboard focus survives a save; the
  // Settings `mutation` guard serialises writes instead.
  assert.doesNotMatch(busy, /disabled=""[^>]*aria-label="Afficher Travail"|aria-label="Afficher Travail"[^>]*disabled=""/);
  assert.match(busy, /<button type="submit" disabled="">Ajouter/);
});
test('Multi: one flat list (name, plan, state, info, visibility switch), then the Add action', () => {
  const current = { ...prefs, claudeAccounts: { mode: 'multi', selectedId: null, profiles: [] } };
  const list = [accounts[0], { ...accounts[1], snapshot: signedOut }, { ...account('Absent'), available: false }];
  const html = renderToStaticMarkup(accountsUx({ prefs: current, accounts: list }));
  assert.doesNotMatch(html, /<details|<form|<p[ >]/);
  assert.equal((html.match(/role="switch"/g) || []).length, 3);
  assert.match(html, /prefs-pill--good">Connecté/);
  assert.match(html, /prefs-pill--warning" title="[^"]*claude auth login[^"]*">Non connecté/);
  assert.match(html, /prefs-pill--neutral" title="[^"]*">Indisponible/);
  assert.equal((html.match(/aria-label="Renommer /g) || []).length, 3);
  assert.equal((html.match(/aria-label="Détail /g) || []).length, 3, 'every account has its detail chevron');
  assert.ok(html.indexOf('Ajouter un compte') > html.lastIndexOf('role="switch"'));
  // The chevron opens the account's detail inline (one level, no sub-panel).
  const open = renderToStaticMarkup(accountsUx({ prefs: current, accounts: list }, ['', null, false, 'Travail']));
  assert.match(open, /aria-expanded="true"[^>]*><svg[\s\S]*<div id="claude-detail-Travail" class="claude-accounts__detail" role="group" aria-label="Détail Travail"><p>Profil non connecté : CLAUDE_CONFIG_DIR=\/fixtures\/x claude auth login\.<\/p><\/div>/);
  assert.equal((open.match(/class="claude-accounts__detail"/g) || []).length, 1, 'only that account');
});
test('inline rename: pencil swaps in a field, Enter saves the trimmed name, Escape cancels', async () => {
  global.window = { requestAnimationFrame: () => 0 };
  const saved = [];
  const props = { account: accounts[1], disabled: false, onSave: async name => { saved.push(name); return true; } };
  const render = (states) => component('../settings/ClaudeAccountsSettings.tsx', props, states, 'AccountName');
  stateChanges = [];
  find(render([]), node => node.props?.['aria-label'] === 'Renommer Travail').props.onClick();
  assert.deepEqual(stateChanges.at(-1), { index: 0, value: 'Travail' });
  const field = find(render(['  Professionnel  ']), node => node.type === 'input');
  assert.equal(field.props.autoFocus, true);
  assert.equal(field.props['aria-label'], 'Nom du compte Travail');
  field.props.onKeyDown({ key: 'Enter', preventDefault() {}, stopPropagation() {} });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(saved, ['Professionnel']);
  assert.deepEqual(stateChanges.at(-1), { index: 0, value: null });
  stateChanges = [];
  find(render(['Autre']), node => node.type === 'input').props.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() {} });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(saved, ['Professionnel']);
  assert.deepEqual(stateChanges, [{ index: 0, value: null }]);
  delete global.window;
});
test('settings add/choose commands are explicit and their failures stay next to the Add action', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = null; invokeResult = prefs;
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, accounts, null, true]);
  const controls = find(tree, node => node.type?.name === 'ClaudeAccountsSettings');
  assert.equal(controls.props.onFollow, undefined);
  invokeResult = '/picked';
  assert.equal(await controls.props.onChooseFolder(), '/picked');
  assert.deepEqual(invokeCalls[0], { command: 'choose_claude_profile_folder', payload: undefined });
  invokeFailure = new Error('« Travail » est déjà affiché.');
  assert.equal(await controls.props.onAdd('/picked'), false);
  assert.deepEqual(invokeCalls[1], { command: 'add_claude_profile', payload: { path: '/picked' } });
  assert.ok(stateChanges.some(change => change.index === 10 && String(change.value).includes('déjà affiché')));
  // No chooser offered where the platform has none (path entry remains).
  const linux = component('../settings/Settings.tsx', {}, ['providers', prefs, null, snapshot, null, false, 0, accounts, null, false]);
  assert.equal(find(linux, node => node.type?.name === 'ClaudeAccountsSettings').props.onChooseFolder, undefined);
  // Fable, usage link and the technical actions stay reachable.
  const html = renderToStaticMarkup(tree);
  assert.match(html, /Quota Fable/);
  assert.match(html, /aria-label="Claude : usage détaillé sur claude\.ai"/);
  assert.ok(controls.props.advanced, 'refresh/relay actions live in the single secondary area');
  invokeFailure = null;
  assert.ok(!invokeCalls.some(call => /refresh|login|follow/.test(call.command)));
});

// ---- Réglages → Affichage : choix visuels -----------------------------------
const display = (p, list = accounts) => component('../settings/Settings.tsx', {}, ['menubar', p, null, snapshot, null, false, 0, list, null, true]);
const group = (html, label) => { const start = html.lastIndexOf("<div", html.indexOf(`aria-label="${label}"`)); return html.slice(start, html.indexOf('</fieldset>', start)); };
test('Affichage: every choice is a labelled radio with a pictogram, and no explanatory paragraph', () => {
  const html = renderToStaticMarkup(display({ ...prefs, layout: 'compact' }));
  const panel = html.slice(html.indexOf('id="panel-menubar"'));
  assert.doesNotMatch(panel, /<p[ >]|Un seul compte|habituel|plusieurs comptes seront/);
  for (const [label, names] of [['Mode d’affichage', ['Barre des menus', 'Application']], ['Indicateur de la barre des menus (quota 5 heures)', ['Codex', 'Claude', 'Les deux']], ['Présentation', ['Compacte', 'Classique']]]) {
    const part = group(html, label);
    assert.match(part, /role="radiogroup"/, label);
    for (const name of names) assert.match(part, new RegExp(`<input type="radio"[^>]*/><span class="prefs-choices__icon" aria-hidden="true"><svg[^>]*>[\\s\\S]*?</svg></span><span>${name}</span>`), name);
  }
  // Presentation: two different schematics, without any figure.
  const presentation = group(html, 'Présentation');
  const svgs = presentation.match(/<svg[\s\S]*?<\/svg>/g);
  assert.equal(svgs.length, 2);
  assert.notEqual(svgs[0], svgs[1]);
  assert.doesNotMatch(presentation, /%|\d+ ?h\b|<text/);
  assert.match(presentation, /<label class="is-selected"><input type="radio" name="layout" checked="" value="compact"/);
  assert.doesNotMatch(presentation, /Multi/);
  // Application mode: the menu-bar indicator has no effect and is not shown.
  assert.doesNotMatch(renderToStaticMarkup(display({ ...prefs, displayMode: 'app' })), /Indicateur de la barre des menus/);
});
test('Classique/Compacte selection reflects the saved choice for one or several accounts, absent = migrated value', () => {
  const one = [accounts[0], { ...accounts[1], visible: false }];
  for (const list of [accounts, one, [accounts[0]], []]) {
    for (const layout of ['compact', 'classic']) {
      assert.match(group(renderToStaticMarkup(display({ ...prefs, layout }, list)), 'Présentation'), new RegExp(`checked="" value="${layout}"`));
    }
  }
  // Unknown value normalised like Rust; absent follows the same migration.
  assert.match(renderToStaticMarkup(display({ ...prefs, layout: 'multi' })), /checked="" value="compact"/);
  assert.match(renderToStaticMarkup(display(prefs)), /checked="" value="classic"/);
  assert.match(renderToStaticMarkup(display({ ...prefs, claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi' } })), /checked="" value="compact"/);
});
test('layoutOf: the single key wins; legacy keys migrate like Rust (Claude explicit > Codex > account mode)', () => {
  const { layoutOf } = require('./providers.ts');
  assert.equal(layoutOf(null), 'classic');
  assert.equal(layoutOf({ claudeAccounts: { mode: 'single' } }), 'classic');
  assert.equal(layoutOf({ claudeAccounts: { mode: 'multi' } }), 'compact');
  assert.equal(layoutOf({ layout: 'classic', claudeLayout: 'compact', codexLayout: 'compact' }), 'classic');
  assert.equal(layoutOf({ layout: 'compact', claudeLayout: 'classic', claudeAccounts: { mode: 'single' } }), 'compact');
  assert.equal(layoutOf({ layout: 'other' }), 'compact');
  assert.equal(layoutOf({ claudeLayout: 'compact', claudeAccounts: { mode: 'single' } }), 'compact');
  assert.equal(layoutOf({ claudeLayout: 'classic', claudeAccounts: { mode: 'multi' } }), 'classic');
  assert.equal(layoutOf({ codexLayout: 'compact' }), 'compact');
  assert.equal(layoutOf({ claudeLayout: 'classic', codexLayout: 'compact' }), 'classic');
  assert.equal(layoutOf({ claudeLayout: 'other' }), 'compact');
});
test('Affichage has ONE Présentation control (name=layout); Fournisseurs has none', () => {
  const html = renderToStaticMarkup(display({ ...prefs, layout: 'classic' }));
  assert.equal((html.match(/name="layout"/g) || []).length, 2);
  assert.doesNotMatch(html, /name="claudeLayout"|name="codexLayout"|Présentation (Codex|Claude|des comptes)/);
  assert.equal((html.match(/<legend[^>]*>Présentation<\/legend>/g) || []).length, 1);
  const providersTab = renderToStaticMarkup(component('../settings/Settings.tsx', {}, ['providers', { ...prefs, codexEnabled: true }, null, snapshot, null, false, 0, accounts, null, true]));
  assert.doesNotMatch(providersTab, /name="layout"|Compacte|Classique|Présentation/);
});
test('one choice changes Claude AND Codex in the Dashboard', () => {
  const codex = [{ model: 'codex', period: 'five-hour', remaining: 63, limit: 100, resetAt: '2030-01-01T12:00:00Z' }, { model: 'codex', period: 'weekly', remaining: 85, limit: 100, resetAt: '2030-01-04T12:00:00Z' }];
  const render = (layout) => {
    global.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
    try { return renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, [codex, null, snapshot, { ...prefs, layout, codexEnabled: true, claudeAccounts: { ...prefs.claudeAccounts, mode: 'single' } }, false, null, null, null, false, 0, [accounts[0]]])); }
    finally { delete global.localStorage; }
  };
  const count = (html) => (html.match(/paired-bars/g) || []).length;
  assert.equal(count(render('compact')), 2, 'Codex + Claude paired');
  assert.equal(count(render('classic')), 0, 'Codex + Claude separated');
});
test('changing the style saves only layout: no refresh, mode or visibility change', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = null;
  const current = { ...prefs, claudeAccounts: { mode: 'multi', selectedId: 'Travail', profiles: [{ id: 'Travail', path: '/fixtures/Travail', name: 'Travail', visible: false }] } };
  invokeResult = { ...current, layout: 'classic' };
  const tree = display(current);
  find(tree, node => node.type?.name === 'Choices' && node.props?.name === 'layout').props.onChange('classic');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(invokeCalls.length, 1);
  assert.deepEqual(invokeCalls[0], { command: 'set_preferences', payload: { prefs: { ...current, layout: 'classic' } } });
  assert.deepEqual(invokeCalls[0].payload.prefs.claudeAccounts, current.claudeAccounts);
});
const dashState = (p, list) => [[], null, snapshot, p, false, null, null, null, false, 0, list];
const metric = (id, used, label, resetAt) => ({ id: 'claude:' + id, label, usedPercent: used, resetAt, availability: 'available', periodDurationSeconds: id === 'five_hour' ? 18000 : 604800, exhausted: false });
const withRows = (a, u5, u7) => ({ ...a, snapshot: { ...snapshot, metrics: [metric('five_hour', u5, 'Limite 5 heures', '2030-01-01T12:00:00Z'), metric('seven_day', u7, 'Limite hebdomadaire', '2030-01-04T12:00:00Z')] } });
const pair = [withRows(accounts[0], 27, 14), withRows(accounts[1], 65, 80)];
const multiPrefs = (layout) => ({ ...prefs, ...(layout ? { layout } : {}), claudeAccounts: { ...prefs.claudeAccounts, mode: 'multi' } });
test('Classique + several accounts: one section per account with separated rows, resets and plans, never aggregated', () => {
  const html = renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, dashState(multiPrefs('classic'), pair)));
  assert.doesNotMatch(html, /paired-bars|provider-card--compact|provider-grid--two/);
  assert.match(html, /aria-label="Quotas Claude · Principal"/);
  assert.match(html, /aria-label="Quotas Claude · Travail"/);
  assert.equal((html.match(/role="progressbar"/g) || []).length, 4);
  assert.equal((html.match(/provider-row__reset/g) || []).length, 4);
  for (const value of ['73 %', '86 %', '35 %', '20 %']) assert.match(html, new RegExp(value));
  assert.equal((html.match(/plan-badge">Pro/g) || []).length, 2);
  assert.match(html, /Limite 5 heures[\s\S]*Limite hebdomadaire[\s\S]*Limite 5 heures[\s\S]*Limite hebdomadaire/);
});
test('Compacte keeps the paired multi cards (absent value from a previous Multi build too)', () => {
  for (const layout of ['compact', undefined]) {
    const html = renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, dashState(multiPrefs(layout), pair)));
    assert.equal((html.match(/provider-card__paired-bars/g) || []).length, 2);
    assert.equal((html.match(/provider-card__window--week/g) || []).length, 2);
  }
});
test('the saved layout changes a sole card too (Unique, Multi with one visible); none visible keeps the empty message', () => {
  const one = [pair[0], { ...pair[1], visible: false }];
  const render = (layout, list, mode = 'multi') => renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {}, dashState({ ...multiPrefs(layout), claudeAccounts: { ...prefs.claudeAccounts, mode } }, list)));
  for (const [list, mode] of [[one, 'multi'], [one, 'single'], [[pair[0]], 'single']]) {
    const compact = render('compact', list, mode);
    const classic = render('classic', list, mode);
    assert.notEqual(compact, classic, mode);
    assert.equal((compact.match(/provider-card__paired-bars/g) || []).length, 1);
    assert.match(compact, /provider-card__window--week/);
    assert.doesNotMatch(classic, /paired-bars/);
    assert.equal((classic.match(/provider-row__reset/g) || []).length, 2);
    for (const html of [compact, classic]) {
      assert.doesNotMatch(html, /Claude · Principal/);
      assert.match(html, /aria-label="Quotas Claude"/);
      assert.match(html, /73 %/);
      assert.match(html, /86 %/);
    }
  }
  const none = pair.map(a => ({ ...a, visible: false }));
  for (const layout of ['classic', 'compact']) {
    const html = render(layout, none);
    assert.match(html, /Aucun compte Claude visible/);
    assert.doesNotMatch(html, /role="progressbar"|%/);
  }
});
