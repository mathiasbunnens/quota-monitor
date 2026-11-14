// Multi-compte Codex côté React : plan des cartes, Dashboard, Réglages.
// Même harnais que claudeProfiles.test.cjs (hooks factices, rendu statique).
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
function find(tree, predicate) {
  if (!tree || typeof tree !== 'object') return null;
  if (predicate(tree)) return tree;
  for (const child of React.Children.toArray(tree.props?.children)) {
    const found = find(child, predicate);
    if (found) return found;
  }
  return null;
}
const providers = () => { hooks = null; return require('./providers.ts'); };

const NOW = Date.parse('2030-01-01T10:00:00Z');
const row = (period, remaining, windowMinutes, resetAt) => ({ model: 'codex', period, remaining, limit: 100, resetAt, checkedAt: '2030-01-01T09:59:00Z', source: 'codex', confidence: 'high', windowMinutes });
const rowsOf = (r5, r7) => [row('codex:codex:primary', r5, 300, '2030-01-01T12:00:00Z'), row('codex:codex:secondary', r7, 10080, '2030-01-04T12:00:00Z')];
const codex = (name, extra = {}) => ({ id: 'profile:' + name, path: '/fixtures/.codex-' + name, name, visible: true, primary: name === 'Principal', tracked: true, available: true, refreshing: false, phase: 'ready', message: 'Quotas synchronisés directement avec Codex.', planType: 'plus', lastChecked: null, rows: rowsOf(63, 85), duplicateOf: null, ...extra });
const principal = codex('Principal');
const travail = codex('Travail', { planType: 'pro', rows: rowsOf(18, 64) });
const perso = codex('Perso', { phase: 'signed_out', planType: null, rows: [], message: 'Non connecté. Connectez ce dossier dans un terminal : CODEX_HOME=/fixtures/.codex-Perso codex login' });
const alias = codex('Alias', { duplicateOf: 'Principal' });
const detected = codex('Detecte', { tracked: false, visible: false, phase: 'detected', rows: [], message: 'Dossier détecté, non suivi.' });
const basePrefs = { codexEnabled: true, claudeEnabled: false, fableEnabled: true, indicator: 'codex', displayMode: 'menubar', ntfy: { enabled: false, serverUrl: '', topic: '' }, claudeAccounts: { mode: 'single', selectedId: null, profiles: [] } };
const status = { phase: 'ready', message: 'Quotas synchronisés directement avec Codex.', executable: null, lastChecked: null, authUrl: null, userCode: null, planType: 'plus', activeInstances: null, refreshSeconds: 120, availableResets: null, refreshSettings: { customSeconds: null } };
const legacyRows = [row('five-hour', 63, 300, '2030-01-01T12:00:00Z'), row('weekly', 85, 10080, '2030-01-04T12:00:00Z')];
// Dashboard useState order: snapshots, status, claude, prefs, showSetup, codexError,
// claudeError, error, refreshing, revision, accounts, now, codexAccounts.
const dash = (prefs, list, snapshots = legacyRows) => {
  global.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
  try {
    return renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {},
      [snapshots, status, null, prefs, false, null, null, null, false, 0, [], NOW, list]));
  } finally { delete global.localStorage; }
};
const multi = (layout, extra = {}) => ({ ...basePrefs, layout, codexAccounts: { mode: 'multi', selectedId: null, profiles: [] }, ...extra });

test('codexPlan: historical card without accounts or with the primary alone; named cards for several; none when disabled', () => {
  const { codexPlan } = providers();
  const legacy = codexPlan(basePrefs, [], legacyRows);
  assert.deepEqual(legacy.cards.map(c => [c.name, c.account, c.rows]), [['Codex', null, legacyRows]]);
  const alone = codexPlan(basePrefs, [principal, { ...travail, visible: false }, detected], legacyRows);
  assert.equal(alone.cards.length, 1);
  assert.equal(alone.cards[0].name, 'Codex');
  assert.equal(alone.cards[0].rows, legacyRows, 'the primary keeps the historical snapshots');
  assert.equal(alone.named, false);
  const several = codexPlan(multi('compact'), [principal, travail, perso, alias, { ...travail, id: 'hidden', name: 'Caché', visible: false }, detected], legacyRows);
  assert.deepEqual(several.cards.map(c => c.name), ['Codex · Principal', 'Codex · Travail', 'Codex · Perso', 'Codex · Alias']);
  assert.equal(several.named, true);
  assert.equal(several.paired, true);
  assert.equal(several.cards[1].rows, travail.rows, 'each card keeps its own rows, nothing is added up');
  // Unique with an additional home selected: one plain « Codex » card with its rows.
  const unique = codexPlan(basePrefs, [{ ...principal, visible: false }, travail], legacyRows);
  assert.deepEqual(unique.cards.map(c => [c.name, c.account.id]), [['Codex', travail.id]]);
  assert.deepEqual(codexPlan({ ...basePrefs, codexEnabled: false }, [principal, travail], legacyRows).cards, []);
  const none = codexPlan(multi('compact'), [{ ...principal, visible: false }, { ...travail, visible: false }], legacyRows);
  assert.deepEqual(none.cards, []);
  assert.equal(none.emptyMulti, true);
});

test('dashboard: one Codex account keeps the exact historical card', () => {
  const before = dash(basePrefs, []);
  const after = dash(basePrefs, [principal, { ...travail, visible: false }, detected]);
  // Only the per-account reset chip and the detail chevron (header) are new; the card itself is the same.
  const stripped = after.replace(/<span class="provider-card__action">[\s\S]*?<\/button><\/span><\/div>/, '').replace('<div class="provider-card__heading">', '')
    .replace(/<button type="button" class="provider-card__chevron"[\s\S]*?<\/button>/, '').replace(/<span class="provider-card__pill[^>]*title="[^"]*"/, m => m.replace(/ title="[^"]*"/, ''));
  assert.equal(stripped, before);
  assert.match(after, /aria-label="Quotas Codex · Principal"/);
});

test('dashboard: several Codex accounts get their own cards, statuses and duplicate note, never aggregated', () => {
  const list = [principal, travail, perso, alias, detected];
  const compact = dash(multi('compact'), list);
  for (const name of ['Principal', 'Travail', 'Perso', 'Alias']) assert.match(compact, new RegExp('aria-label="Quotas Codex · ' + name + '"'));
  assert.doesNotMatch(compact, /Detecte/);
  assert.equal((compact.match(/provider-card__window--short/g) || []).length, 3, 'three cards with real windows are paired');
  assert.equal((compact.match(/provider-card__window--week/g) || []).length, 3);
  for (const value of ['63 %', '85 %', '18 %', '64 %']) assert.match(compact, new RegExp(value));
  assert.doesNotMatch(compact, /81 %|149 %|Total/);
  assert.match(compact, /Non connecté\. Connectez ce dossier dans un terminal : CODEX_HOME=\/fixtures\/\.codex-Perso codex login/);
  assert.match(compact, /Même compte que « Principal » : quotas non additionnés\./);
  assert.match(compact, /plan-badge">pro/);
  const classic = dash(multi('classic'), list);
  assert.doesNotMatch(classic, /paired-bars/);
  assert.equal((classic.match(/provider-row__reset/g) || []).length, 6);
  // Unavailable/refreshing/error stay on their own card.
  const states = dash(multi('compact'), [principal, { ...travail, available: false, phase: 'unavailable', rows: [] }, { ...alias, duplicateOf: null, phase: 'error', rows: [], message: 'Codex a quitté.' }, { ...perso, refreshing: true }]);
  assert.match(states, /Dossier indisponible\./);
  assert.match(states, /Codex a quitté\./);
  assert.match(states, /Actualisation…/);
  assert.match(states, /aria-label="Quotas Codex · Principal"[\s\S]*63 %/);
});

test('dashboard: Multi with every Codex account hidden shows the empty message; disabled Codex shows nothing', () => {
  const hidden = dash(multi('compact'), [{ ...principal, visible: false }, { ...travail, visible: false }]);
  assert.match(hidden, /Aucun compte Codex visible/);
  assert.doesNotMatch(hidden, /role="progressbar"/);
  const off = dash({ ...multi('compact'), codexEnabled: false, claudeEnabled: true }, [principal, travail]);
  assert.doesNotMatch(off, /Quotas Codex/);
});

// --- Réglages -----------------------------------------------------------------
const accountsProps = (overrides = {}) => ({ prefs: multi('compact'), accounts: [principal, travail, detected], busy: false,
  onUpdate: async () => true, onAdd: async () => true, onUntrack: async () => true, ...overrides });
const codexUx = (props, states = []) => {
  // The wrapper only binds the Codex wording/state; render the shared list it returns.
  const element = component('../settings/ClaudeAccountsSettings.tsx', accountsProps(props), [], 'CodexAccountsSettings');
  assert.equal(element.props.provider, 'Codex');
  return component('../settings/ClaudeAccountsSettings.tsx', element.props, states);
};

test('Codex settings reuse the Claude list: Unique/Multi, visibility switches, an auto-followed folder is a plain account', () => {
  const html = renderToStaticMarkup(codexUx({}));
  assert.match(html, /aria-label="Comptes Codex affichés"/);
  assert.match(html, /name="codex-mode"/);
  assert.match(html, /aria-label="Comptes Codex"/);
  assert.match(html, /class="claude-accounts__row"/);
  assert.match(html, /role="switch"[^>]*aria-label="Afficher Codex Principal"|aria-label="Afficher Codex Principal"[^>]*role="switch"/);
  assert.match(html, /aria-label="Afficher Codex Travail"/);
  assert.match(html, /aria-label="Afficher Codex Detecte"/, 'a detected folder is followed automatically, so it has its switch');
  assert.doesNotMatch(html, /Suivre|>Détecté</, 'no Détecté → Suivre step');
  assert.match(html, /aria-label="Ajouter un compte Codex"/);
  assert.doesNotMatch(html, />Ajouter un compte</);
  assert.doesNotMatch(html, /claude-mode|claude-selected|Claude Code/);
});

test('Codex settings: two followed homes are needed for Unique/Multi; states are explicit', () => {
  const one = renderToStaticMarkup(codexUx({ prefs: basePrefs, accounts: [principal] }));
  assert.doesNotMatch(one, /codex-mode/);
  const states = renderToStaticMarkup(codexUx({ accounts: [principal, perso, alias, { ...travail, available: false, phase: 'unavailable' }] }));
  assert.match(states, />Non connecté</);
  assert.match(states, />Même compte</);
  assert.match(states, />Indisponible</);
  const { codexAccountState } = require('../settings/ClaudeAccountsSettings.tsx');
  assert.deepEqual(codexAccountState(perso).slice(0, 2), ['Non connecté', 'warning']);
  assert.match(codexAccountState(perso)[2], /CODEX_HOME=\/fixtures\/\.codex-Perso codex login/);
  assert.match(codexAccountState(alias)[2], /Même compte que « Principal »/);
  assert.equal(codexAccountState({ ...principal, phase: 'disabled' })[0], 'Désactivé');
});

// ---- aip : home Codex incomplet (non collectable) et home expiré ----
test('a blocked aip Codex home reads « Non collectable » with its reason, never as an error', () => {
  const { codexAccountState } = require('../settings/ClaudeAccountsSettings.tsx');
  const message = 'Profil aip incomplet (aucun auth.json) : rien à lire.';
  const blocked = { ...perso, name: 'boulot', path: '/fixtures/.codex/profiles/boulot', tracked: false, phase: 'blocked', message, planType: null, rows: [] };
  assert.deepEqual(codexAccountState(blocked), ['Non collectable', 'neutral', message]);
  assert.equal(codexAccountState({ ...blocked, message: '' })[0], 'Non collectable');
  assert.equal(codexAccountState({ ...blocked, available: false })[0], 'Indisponible');
  // An aip home read with its own token behaves like any followed home (tracked,
  // ready with rows) and reads « Session expirée » when its token no longer works.
  const expired = { ...perso, name: 'perso', path: '/fixtures/.codex/profiles/perso', tracked: true, phase: 'expired', message: 'Session du profil aip « perso » expirée : lancez « aip codex ».', planType: null, rows: [] };
  assert.deepEqual(codexAccountState(expired), ['Session expirée', 'warning', expired.message]);
  assert.equal(codexAccountState({ ...expired, message: '' })[0], 'Session expirée');
  assert.equal(codexAccountState({ ...expired, available: false })[0], 'Indisponible');
  const html = renderToStaticMarkup(codexUx({ accounts: [principal, blocked] }));
  assert.match(html, />Non collectable</);
  assert.doesNotMatch(html, />Erreur</);
  // Nothing to display or select: no visibility switch / Unique pick for it.
  assert.doesNotMatch(html, /aria-label="Afficher Codex boulot"/);
  const multiHtml = renderToStaticMarkup(codexUx({ prefs: { ...basePrefs, codexAccounts: { mode: 'multi', selectedId: null, profiles: [] } }, accounts: [principal, travail, blocked] }));
  assert.match(multiHtml, /aria-label="Afficher Codex Travail"/);
  assert.doesNotMatch(multiHtml, /aria-label="Afficher Codex boulot"/);
  assert.match(multiHtml, />Non collectable</);
});

test('Codex settings edits only codexAccounts (visibility, selection, rename); no removal or detail on a row', async () => {
  let patch;
  const tree = codexUx({ onUpdate: async value => { patch = value; return true; } }, ['', null, true]);
  find(tree, node => node.props?.['aria-label'] === 'Afficher Codex Travail').props.onChange({ target: { checked: false } });
  assert.deepEqual(Object.keys(patch), ['codexAccounts']);
  assert.deepEqual(patch.codexAccounts.profiles, [{ id: travail.id, path: travail.path, name: 'Travail', visible: false }]);
  await find(tree, node => node.type?.name === 'AccountName').props.onSave('Maison');
  assert.deepEqual(Object.keys(patch), ['codexAccounts']);
  assert.equal(patch.codexAccounts.profiles[0].name, 'Maison');
  // Secondary area (open) keeps only the manual path field; rows carry neither path nor Supprimer.
  const html = renderToStaticMarkup(tree);
  assert.doesNotMatch(html, /Ne plus suivre|Suivre/);
  assert.match(html, /placeholder="\/Users\/nom\/\.codex-travail"/);
  assert.doesNotMatch(html, /Supprimer|claude-accounts__detail|claude-accounts__path|aria-label="Détail/);
  // Unique selection.
  const single = codexUx({ prefs: basePrefs, onUpdate: async value => { patch = value; return true; } });
  find(single, node => node.props?.['aria-label'] === 'Afficher Codex Travail').props.onChange();
  assert.deepEqual(patch, { codexAccounts: { mode: 'single', selectedId: travail.id, profiles: [] } });
});

// Settings useState order: tab, prefs, status, claude, error, busy, revision,
// accounts, accountsError, canChooseMode, accountActionError, codexAccounts,
// codexAccountsError, codexActionError.
const settingsState = (prefs, list) => ['providers', prefs, status, null, null, false, 0, [], null, true, null, list, null, null];

test('Settings: Codex accounts block uses the explicit add command; old prefs without codexAccounts default to Unique', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = null;
  const tree = component('../settings/Settings.tsx', {}, settingsState(basePrefs, [principal, travail]));
  const block = find(tree, node => node.type?.name === 'CodexAccountsSettings');
  assert.ok(block, 'Codex accounts block present in Fournisseurs');
  assert.deepEqual(block.props.prefs.codexAccounts ?? { mode: 'single', selectedId: null, profiles: [] }, { mode: 'single', selectedId: null, profiles: [] });
  invokeResult = basePrefs;
  assert.equal(await block.props.onAdd('/fixtures/.codex-new'), true);
  assert.equal(block.props.onFollow, undefined, 'detected homes are followed by the backend, no Follow step');
  assert.equal(block.props.onUntrack, undefined, 'no removal from the account rows');
  invokeResult = '/fixtures/.codex-picked';
  assert.equal(await block.props.onChooseFolder(), '/fixtures/.codex-picked');
  assert.deepEqual(invokeCalls.map(c => [c.command, c.payload]), [
    ['add_codex_profile', { path: '/fixtures/.codex-new' }],
    ['choose_codex_profile_folder', undefined],
  ]);
  assert.ok(!invokeCalls.some(c => /login|logout|claude/.test(c.command)), 'no login, logout or Claude call');
  // The Claude block is still the Claude one.
  assert.equal(find(tree, node => node.type?.name === 'ClaudeAccountsSettings').props.provider, undefined);
  invokeFailure = new Error('Dossier introuvable');
  assert.equal(await block.props.onAdd('/missing'), false);
  assert.ok(stateChanges.some(change => change.index === 13 && String(change.value).includes('Dossier introuvable')));
  invokeFailure = null;
});

test('Settings polls get_codex_accounts and reports a discovery failure without touching Claude', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = new Error('fixture');
  global.window = { setTimeout: () => 1, clearTimeout() {} };
  component('../settings/Settings.tsx', {}, settingsState(basePrefs, []));
  const cleanup = effects.find(effect => String(effect).includes('get_codex_accounts'))();
  await new Promise(resolve => setImmediate(resolve));
  cleanup();
  invokeFailure = null;
  delete global.window;
  assert.ok(invokeCalls.some(call => call.command === 'get_codex_accounts'));
  assert.ok(stateChanges.some(change => change.index === 12 && String(change.value).includes('comptes Codex')));
});

test('Dashboard polls get_codex_accounts alongside the historical snapshots', async () => {
  invokeCalls = []; stateChanges = []; invokeFailure = null; invokeResult = [];
  global.window = { setTimeout: () => 1, clearTimeout() {}, setInterval: () => 1, clearInterval() {} };
  component('../../pages/Dashboard/Dashboard.tsx', {}, [legacyRows, status, null, basePrefs, false, null, null, null, false, 0, [], NOW, []]);
  const cleanup = effects.find(effect => String(effect).includes('get_codex_accounts'))();
  await new Promise(resolve => setImmediate(resolve));
  cleanup();
  delete global.window;
  const commands = invokeCalls.map(c => c.command);
  assert.ok(commands.includes('get_codex_accounts') && commands.includes('get_quota_snapshots'));
  assert.ok(!commands.some(c => /login|logout|refresh/.test(c)), 'polling never logs in or forces a refresh');
});
