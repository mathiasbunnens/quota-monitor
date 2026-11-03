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
function component(file, props, states = [], exportName = 'default', purge = true) {
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
  for (const cached of purge ? Object.keys(require.cache) : []) {
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

// ---------------------------------------------------------------------------
// Icônes de réinitialisation / connexion / usage : sur chaque compte, plus dans
// l'en-tête fournisseur. Même harnais que codexAccounts.test.cjs.
// ---------------------------------------------------------------------------
const NOW = Date.parse('2030-01-01T10:00:00Z');
const row = (period, remaining, windowMinutes, resetAt) => ({ model: 'codex', period, remaining, limit: 100, resetAt, checkedAt: '2030-01-01T09:59:00Z', source: 'codex', confidence: 'high', windowMinutes });
const rowsOf = (r5, r7) => [row('codex:codex:primary', r5, 300, '2030-01-01T12:00:00Z'), row('codex:codex:secondary', r7, 10080, '2030-01-04T12:00:00Z')];
const codex = (name, extra = {}) => ({ id: 'profile:' + name, path: '/fixtures/.codex-' + name, name, visible: true, primary: name === 'Principal', tracked: true, available: true, refreshing: false, phase: 'ready', message: 'Quotas synchronisés directement avec Codex.', planType: 'plus', lastChecked: null, rows: rowsOf(63, 85), duplicateOf: null, availableResets: 2, ...extra });
const principal = codex('Principal');
const travail = codex('Travail', { planType: 'pro', rows: rowsOf(18, 64), availableResets: 1 });
const vide = codex('Vide', { availableResets: 0 });
const perso = codex('Perso', { phase: 'signed_out', planType: null, rows: [], availableResets: null, message: 'Non connecté. Connectez ce dossier dans un terminal : CODEX_HOME=/fixtures/.codex-Perso codex login' });
const alias = codex('Alias', { duplicateOf: 'Principal' });
const metric = (id, label, used) => ({ id, label, usedPercent: used, resetAt: '2030-01-01T12:00:00Z', availability: 'available', periodDurationSeconds: 18000, exhausted: false });
const claudeSnap = (extra = {}) => ({ provider: 'claude', schemaVersion: 1, enabled: true, configured: true, state: 'ready', source: null, observedAt: null, providerFetchedAt: null, message: '', errorKind: null, metrics: [metric('claude:five_hour', 'Limite 5 heures', 20), metric('claude:seven_day', 'Limite hebdomadaire', 40)], capability: { refresh: 'on_demand' }, planType: 'Pro', extraUsage: null, ...extra });
const claudeAccount = (name, extra = {}) => ({ id: 'claude:' + name, path: '/fixtures/.claude-' + name, name, visible: true, primary: name === 'Principal', available: true, refreshing: false, snapshot: claudeSnap(), ...extra });
const status = { phase: 'ready', message: 'ok', executable: null, lastChecked: null, authUrl: null, userCode: null, planType: 'plus', activeInstances: null, refreshSeconds: 120, availableResets: 2, refreshSettings: { customSeconds: null } };
const basePrefs = { codexEnabled: true, claudeEnabled: true, fableEnabled: true, indicator: 'codex', displayMode: 'menubar', ntfy: { enabled: false, serverUrl: '', topic: '' }, claudeAccounts: { mode: 'single', selectedId: null, profiles: [] } };
const multiPrefs = { ...basePrefs, layout: 'compact', codexAccounts: { mode: 'multi', selectedId: null, profiles: [] } };
const store = {};
const withStorage = (fn) => {
  global.localStorage = { getItem: key => store[key] ?? null, setItem: (key, value) => { store[key] = String(value); }, removeItem: key => { delete store[key]; } };
  return fn();
};
const tick = () => new Promise(resolve => setImmediate(resolve));

// ResetCredits state order: busy, pending, message, confirming, done.
const chip = (props, states = [], purge = true) => withStorage(() => component('./ResetCredits.tsx', { accountId: principal.id, accountName: 'Principal', primary: true, ready: true, count: 2, onChange() {}, ...props }, states, 'default', purge));

test('reset chip: named after its account, enabled only with a credit, disabled otherwise', () => {
  const on = renderToStaticMarkup(chip({ accountName: 'Travail', count: 1 }));
  assert.match(on, /aria-label="Réinitialisations disponibles : 1 — Travail"/);
  assert.match(on, /title="Utiliser une réinitialisation"/);
  assert.doesNotMatch(on, /disabled=""/);
  for (const off of [{ count: 0 }, { count: null }, { ready: false, count: 3 }]) {
    const html = renderToStaticMarkup(chip({ accountName: 'Travail', ...off }));
    assert.match(html, /class="reset-chip" disabled=""/);
    assert.match(html, /title="Aucune réinitialisation disponible"/);
  }
});

test('reset confirmation names the account and spends the reset on that account id only', async () => {
  invokeCalls = []; invokeFailure = null; invokeResult = { outcome: 'reset', quotasRefreshed: true };
  const dialog = chip({ accountId: travail.id, accountName: 'Travail', primary: false, count: 1 }, [false, null, '', true, false]);
  const html = renderToStaticMarkup(dialog);
  assert.match(html, /compte Codex « Travail »/);
  const confirm = find(dialog, node => node.props?.className === 'primary');
  await withStorage(() => confirm.props.onClick());
  await tick();
  assert.equal(invokeCalls.length, 1);
  assert.equal(invokeCalls[0].command, 'redeem_codex_reset');
  assert.equal(invokeCalls[0].payload.accountId, travail.id);
  assert.match(invokeCalls[0].payload.idempotencyKey, /^[0-9a-f-]{36}$/);
});

test('a lost reply keeps a pending key per account: another account never reuses it, the primary keeps its historical key', async () => {
  invokeCalls = []; invokeResult = null; invokeFailure = new Error('réseau');
  for (const key of Object.keys(store)) delete store[key];
  const confirm = (props) => find(chip({ ...props }, [false, null, '', true, false]), node => node.props?.className === 'primary');
  await withStorage(() => confirm({ accountId: travail.id, accountName: 'Travail', primary: false, count: 1 }).props.onClick());
  await tick();
  assert.deepEqual(Object.keys(store), ['codex-pending-reset:' + travail.id]);
  const travailKey = store['codex-pending-reset:' + travail.id];
  await withStorage(() => confirm({ accountId: principal.id, accountName: 'Principal', primary: true, count: 2 }).props.onClick());
  await tick();
  assert.deepEqual(Object.keys(store).sort(), ['codex-pending-reset', 'codex-pending-reset:' + travail.id]);
  assert.notEqual(store['codex-pending-reset'], travailKey);
  invokeFailure = null;
});

test('a reset in flight blocks a double click and any concurrent reset on another account', async () => {
  invokeCalls = []; invokeFailure = null;
  let release;
  invokeResult = new Promise(resolve => { release = resolve; });
  const button = find(chip({ accountId: travail.id, accountName: 'Travail', primary: false, count: 1 }, [false, null, '', true, false]), node => node.props?.className === 'primary');
  const { redeemLock } = require('./ResetCredits.tsx');
  const first = button.props.onClick();
  const second = button.props.onClick();
  await tick();
  assert.equal(invokeCalls.length, 1, 'double click ignored');
  assert.equal(redeemLock.acquire(), false, 'another account cannot start meanwhile');
  release({ outcome: 'reset', quotasRefreshed: true });
  await Promise.all([first, second]);
  assert.equal(redeemLock.acquire(), true, 'released once the reply is in');
  redeemLock.release();
  invokeResult = null;
  for (const key of Object.keys(store)) delete store[key];
});

// ---- Réglages : Fournisseurs -------------------------------------------------
// Settings useState order: tab, prefs, status, claude, error, busy, revision,
// accounts, accountsError, canChooseMode, accountActionError, codexAccounts,
// codexAccountsError, codexActionError.
const settingsHtml = (prefs, codexList, claudeList = [], extra = {}) => renderToStaticMarkup(component('../settings/Settings.tsx', {},
  ['providers', prefs, extra.status ?? status, null, null, false, 0, claudeList, null, true, null, codexList, null, null]));
const section = (html, id) => { const start = html.indexOf(`aria-labelledby="${id}"`); return html.slice(start, html.indexOf('</section>', start)); };
const header = (html, id) => { const block = section(html, id); return block.slice(0, block.indexOf('</header>')); };
const rows = (html) => html.split('<li class="claude-accounts__row"').slice(1);

test('provider headers keep only the provider: title with the usage arrow, enable switch; no state, plan, reset or usage row', () => {
  const html = settingsHtml({ ...multiPrefs, claudeAccounts: { mode: 'multi', selectedId: null, profiles: [] } }, [principal, travail],
    [claudeAccount('Principal', { snapshot: claudeSnap({ extraUsage: { enabled: true, limitReached: false } }) }), claudeAccount('Travail')]);
  for (const [id, name, command] of [['codex-heading', 'Codex', 'open_usage_details'], ['claude-heading', 'Claude', 'open_claude_usage']]) {
    const head = header(html, id);
    assert.match(head, new RegExp('<h2 id="' + id + '" class="prefs-group__title"><button type="button" class="prefs-title-link"[^>]*>' + name + ' <svg class="external-icon"'));
    assert.doesNotMatch(head, /prefs-pill|reset-chip|prefs-plan|Connecté|Réinitialisations/);
    assert.match(head, /role="switch"/);
  }
  assert.doesNotMatch(html, /Usage détaillé sur (chatgpt\.com|claude\.ai)<svg|prefs-links/, 'no separate « Usage détaillé » row');
  assert.match(html, /aria-label="Codex : usage détaillé sur chatgpt\.com"/);
  assert.match(html, /aria-label="Claude : usage détaillé sur claude\.ai"/);
});

test('the title arrow is the existing external-link icon, not a chevron, and opens the provider usage page', async () => {
  invokeCalls = []; invokeFailure = null; invokeResult = null;
  const tree = component('../settings/Settings.tsx', {}, ['providers', multiPrefs, status, null, null, false, 0, [claudeAccount('Principal')], null, true, null, [principal], null, null]);
  const links = [];
  const walk = (node) => { if (!node || typeof node !== 'object') return; if (node.props?.className === 'prefs-title-link') links.push(node); React.Children.toArray(node.props?.children).forEach(walk); };
  walk(tree);
  assert.equal(links.length, 2);
  const icon = require('../ui/ExternalIcon.tsx').default;
  for (const link of links) assert.ok(find(link, node => node.type === icon), 'ExternalIcon is reused');
  links.forEach(link => link.props.onClick());
  await tick();
  assert.deepEqual(invokeCalls.map(call => call.command), ['open_usage_details', 'open_claude_usage']);
});

test('each Codex account row carries its own reset chip, state and detail chevron, then its visibility switch', () => {
  const html = settingsHtml(multiPrefs, [principal, travail, vide, perso, alias]);
  const list = rows(section(html, 'codex-heading'));
  assert.equal(list.length, 5);
  const order = (item) => ['reset-chip', 'prefs-pill', 'aria-label="Détail', 'role="switch"'].map(token => item.indexOf(token));
  // Principal (2 resets), Travail (1), Vide (0, disabled chip).
  assert.match(list[0], /aria-label="Réinitialisations disponibles : 2 — Principal"/);
  assert.match(list[1], /aria-label="Réinitialisations disponibles : 1 — Travail"/);
  assert.match(list[2], /class="reset-chip" disabled=""[^>]*aria-label="Réinitialisations disponibles : 0 — Vide"|aria-label="Réinitialisations disponibles : 0 — Vide"[^>]*disabled=""/);
  for (const item of list.slice(0, 3)) {
    const [chipAt, pillAt, chevronAt, switchAt] = order(item);
    assert.ok(chipAt > 0 && chipAt < pillAt && pillAt < chevronAt && chevronAt < switchAt, 'order: reset, connected, chevron, switch');
    assert.match(item, /prefs-pill--good">Connecté</);
  }
  // Signed-out and duplicate accounts: no reset icon at all, their own state.
  assert.doesNotMatch(list[3], /reset-chip/);
  assert.match(list[3], />Non connecté</);
  assert.doesNotMatch(list[4], /reset-chip/);
  assert.match(list[4], />Même compte</);
  // Detail chevron, named with provider and account.
  assert.match(list[1], /aria-label="Détail Codex Travail"[^>]*>/);
});

test('a single account has the same placement; Claude has a state and chevron but never a reset', () => {
  const html = settingsHtml(basePrefs, [principal], [claudeAccount('Principal', { snapshot: claudeSnap({ extraUsage: { enabled: true, limitReached: true } }) })]);
  const codexRow = rows(section(html, 'codex-heading'));
  assert.equal(codexRow.length, 1);
  assert.match(codexRow[0], /reset-chip[\s\S]*prefs-pill--good">Connecté[\s\S]*aria-label="Détail Codex Principal"/);
  const claudeRow = rows(section(html, 'claude-heading'));
  assert.equal(claudeRow.length, 1);
  assert.doesNotMatch(claudeRow[0], /Réinitialisations disponibles/);
  assert.match(claudeRow[0], /prefs-pill--good">Connecté[\s\S]*aria-label="Détail Principal"/);
  // Extra usage (kept) now belongs to the account that has it.
  assert.match(claudeRow[0], /aria-label="Usage supplémentaire Claude : Principal"[\s\S]*Plafond/);
  assert.doesNotMatch(header(html, 'claude-heading'), /Plafond|Extra/);
});

test('the manual add is a single + icon button at the end of every provider list, named for the provider, even with one account', () => {
  for (const [codexList, claudeList] of [[[principal], [claudeAccount('Principal')]], [[principal, travail], [claudeAccount('Principal'), claudeAccount('Travail')]]]) {
    const html = settingsHtml(multiPrefs, codexList, claudeList);
    for (const [id, provider] of [['codex-heading', 'Codex'], ['claude-heading', 'Claude']]) {
      const block = section(html, id);
      const button = block.match(/<button[^>]*claude-accounts__add[^>]*>[\s\S]*?<\/button>/);
      assert.ok(button, 'the + button exists');
      assert.match(button[0], new RegExp('aria-label="Ajouter un compte ' + provider + '"'));
      assert.match(button[0], /<svg[^>]*><path d="M8 3\.5v9M3\.5 8h9"><\/path><\/svg><\/button>$/, 'icon only, the existing plus glyph');
      assert.equal((block.match(/claude-accounts__add/g) || []).length, 1);
      assert.doesNotMatch(block, />\s*Ajouter un compte\s*</, 'no « Ajouter un compte » line');
      assert.ok(block.indexOf('claude-accounts__add') > block.lastIndexOf('claude-accounts__row'));
    }
  }
});

test('the chevron opens only that account, inline: its folder and « Supprimer » and nothing else', () => {
  // CodexAccountsSettings state: path, pathError, open, details.
  const props = { prefs: multiPrefs, accounts: [principal, travail, perso], busy: false, onUpdate: async () => true, onAdd: async () => true, onUntrack: async () => true };
  const open = (id, extra = {}) => {
    const element = component('../settings/ClaudeAccountsSettings.tsx', { ...props, ...extra }, [], 'CodexAccountsSettings');
    return renderToStaticMarkup(component('../settings/ClaudeAccountsSettings.tsx', element.props, ['', null, false, id]));
  };
  const travailOpen = open(travail.id);
  const detail = travailOpen.match(/<div id="codex-detail-profile:Travail"[\s\S]*?<\/div>/)[0];
  assert.match(detail, /<code class="claude-accounts__path" title="\/fixtures\/\.codex-Travail">\/fixtures\/\.codex-Travail<\/code>/);
  assert.match(detail, />Supprimer Travail<\/button>/);
  assert.equal(detail.replace(/<[^>]+>/g, '|').split('|').filter(Boolean).join(' / '), '/fixtures/.codex-Travail / Supprimer Travail', 'path + Supprimer only');
  assert.doesNotMatch(travailOpen, /% restants|Réinitialisation :|Quotas synchronisés|<p>|prefs-pill[^>]*>[^<]*<\/span><\/div>/);
  assert.equal((travailOpen.match(/aria-expanded="true"/g) || []).length, 1);
  assert.doesNotMatch(travailOpen, /\.codex-Principal<\/code>|\.codex-Perso<\/code>/, 'never the folder of another account');
  // Principal: the path only, never a Supprimer.
  const principalOpen = open(principal.id);
  assert.match(principalOpen, /id="codex-detail-profile:Principal"/);
  assert.doesNotMatch(principalOpen, /Supprimer/);
  // Signed-out: same minimal detail; its state stays on the row (« Non connecté »).
  const persoOpen = open(perso.id);
  assert.doesNotMatch(persoOpen.match(/<div id="codex-detail-profile:Perso"[\s\S]*?<\/div>/)[0], /codex login|Non connecté|% restants/, 'the state stays on the row, not in the detail');
  assert.match(persoOpen, />Non connecté</);
  assert.doesNotMatch(open(null), /claude-accounts__detail/);
  // Without a removal handler (nothing to remove): the path only.
  assert.doesNotMatch(open(travail.id, { onUntrack: undefined }), /Supprimer/);
});

test('Supprimer calls the untrack handler with the account id only (preference, never a file)', async () => {
  const removed = [];
  const element = component('../settings/ClaudeAccountsSettings.tsx', { prefs: multiPrefs, accounts: [principal, travail], busy: false, onUpdate: async () => true, onAdd: async () => true, onUntrack: async id => { removed.push(id); return true; } }, [], 'CodexAccountsSettings');
  global.window = { requestAnimationFrame: fn => { fn(); return 1; } };
  const tree = component('../settings/ClaudeAccountsSettings.tsx', element.props, ['', null, false, travail.id]);
  const button = find(tree, node => node.props?.className?.includes('claude-accounts__remove'));
  assert.ok(button && !find(tree, node => node.props?.id === 'codex-detail-profile:Principal'));
  await button.props.onClick();
  delete global.window;
  assert.deepEqual(removed, [travail.id]);
});

test('each provider routes its + and Supprimer to its own commands; neither touches the other provider or a login', async () => {
  invokeCalls = []; invokeFailure = null; invokeResult = multiPrefs;
  const tree = component('../settings/Settings.tsx', {}, ['providers', multiPrefs, status, null, null, false, 0, [claudeAccount('Principal'), claudeAccount('Travail')], null, true, null, [principal, travail], null, null]);
  const codexBlock = find(tree, node => node.type?.name === 'CodexAccountsSettings');
  const claudeBlock = find(tree, node => node.type?.name === 'ClaudeAccountsSettings');
  assert.equal(await codexBlock.props.onAdd('/fixtures/.codex-new'), true);
  assert.equal(await claudeBlock.props.onAdd('/fixtures/.claude-new'), true);
  assert.equal(await codexBlock.props.onUntrack(travail.id), true);
  assert.equal(await claudeBlock.props.onUntrack('claude:Travail'), true);
  assert.deepEqual(invokeCalls.map(c => [c.command, c.payload]), [
    ['add_codex_profile', { path: '/fixtures/.codex-new' }],
    ['add_claude_profile', { path: '/fixtures/.claude-new' }],
    ['untrack_codex_profile', { id: travail.id }],
    ['untrack_claude_profile', { id: 'claude:Travail' }],
  ]);
  assert.ok(!invokeCalls.some(c => /login|logout|delete|remove_file/.test(c.command)));
  // The + opens each provider's own folder chooser.
  invokeCalls = []; invokeResult = '/fixtures/picked';
  assert.equal(await codexBlock.props.onChooseFolder(), '/fixtures/picked');
  assert.equal(await claudeBlock.props.onChooseFolder(), '/fixtures/picked');
  assert.deepEqual(invokeCalls.map(c => c.command), ['choose_codex_profile_folder', 'choose_claude_profile_folder']);
  invokeResult = null;
});

// ---- Dashboard -----------------------------------------------------------------
// Dashboard useState order: snapshots, status, claude, prefs, showSetup, codexError,
// claudeError, error, refreshing, revision, accounts, now, codexAccounts.
const legacyRows = [row('five-hour', 63, 300, '2030-01-01T12:00:00Z'), row('weekly', 85, 10080, '2030-01-04T12:00:00Z')];
const dash = (prefs, list, claudeList = []) => withStorage(() => renderToStaticMarkup(component('../../pages/Dashboard/Dashboard.tsx', {},
  [legacyRows, status, null, prefs, false, null, null, null, false, 0, claudeList, NOW, list])));
const cards = (html) => html.split('<section class="provider-card').slice(1).map(card => card.slice(0, card.indexOf('</section><section') > 0 ? card.indexOf('</section><section') : undefined));

test('dashboard: the reset chip sits in the header of each Codex account card, named, and not after the grid', () => {
  const html = dash({ ...multiPrefs, claudeEnabled: false }, [principal, travail, perso, alias]);
  const list = cards(html);
  assert.equal(list.length, 4);
  assert.match(list[0], /<header class="provider-card__header">[\s\S]*aria-label="Réinitialisations disponibles : 2 — Principal"[\s\S]*<\/header>/);
  assert.match(list[1], /aria-label="Réinitialisations disponibles : 1 — Travail"/);
  assert.doesNotMatch(list[2], /reset-chip/, 'signed-out account: nothing');
  assert.doesNotMatch(list[3], /reset-chip/, 'duplicate account: nothing');
  assert.equal((html.match(/class="reset-chip"/g) || []).length, 2, 'no block outside the cards');
  assert.doesNotMatch(html, /reset-credits|Réinitialisations supplémentaires/, 'the former separate section is gone');
});

test('dashboard: one Codex account keeps its reset on the card, in the same place', () => {
  const html = dash(basePrefs, [principal]);
  assert.match(cards(html)[0], /<header class="provider-card__header">[\s\S]*aria-label="Réinitialisations disponibles : 2 — Principal"/);
  assert.doesNotMatch(dash({ ...basePrefs, claudeEnabled: false }, [perso]), /reset-chip/);
});

test('dashboard: the usage arrow is on the provider title, the separate footer link is gone', () => {
  const html = dash(multiPrefs, [principal, travail], [claudeAccount('Principal')]);
  assert.equal((html.match(/class="provider-card__usage"/g) || []).length, 2, 'once for Codex, once for Claude');
  assert.match(html, /aria-label="Codex : usage détaillé sur chatgpt\.com"/);
  assert.match(html, /aria-label="Claude : usage détaillé sur claude\.ai"/);
  assert.doesNotMatch(html, /Voir les détails sur Codex/);
});
