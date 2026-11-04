// Finition Réglages (volet A) : un seul +, parcours d'ajout unique, un seul « Principal »,
// accès à la connexion Codex depuis le détail du principal.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { renderToStaticMarkup } = require('react-dom/server');
const { env, component, find } = require('./harness.cjs');

const row = (period, remaining, windowMinutes, resetAt) => ({ model: 'codex', period, remaining, limit: 100, resetAt, checkedAt: '2030-01-01T09:59:00Z', source: 'codex', confidence: 'high', windowMinutes });
const rowsOf = (r5, r7) => [row('codex:codex:primary', r5, 300, '2030-01-01T12:00:00Z'), row('codex:codex:secondary', r7, 10080, '2030-01-04T12:00:00Z')];
const codex = (name, extra = {}) => ({ id: name === 'Principal' ? 'primary' : 'profile:' + name, path: '/fixtures/.codex-' + name, name, visible: true, primary: name === 'Principal', tracked: true, available: true, refreshing: false, phase: 'ready', message: '', planType: 'plus', lastChecked: null, rows: rowsOf(63, 85), duplicateOf: null, availableResets: 2, ...extra });
const principal = codex('Principal');
const travail = codex('Travail');
const status = { phase: 'ready', message: 'ok', executable: '/usr/local/bin/codex', lastChecked: null, authUrl: null, userCode: null, planType: 'plus', activeInstances: null, refreshSeconds: 120, availableResets: 2, refreshSettings: { customSeconds: null } };
const prefs = { codexEnabled: true, claudeEnabled: false, fableEnabled: true, indicator: 'codex', displayMode: 'menubar', ntfy: { enabled: false, serverUrl: '', topic: '' }, claudeAccounts: { mode: 'single', selectedId: null, profiles: [] }, layout: 'compact', codexAccounts: { mode: 'multi', selectedId: null, profiles: [] } };
global.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
const text = (html) => html.replace(/<[^>]+>/g, '|').split('|').map(s => s.trim()).filter(Boolean);

const list = (props = {}, states = ['', null, false, null]) => {
  const element = component('../settings/ClaudeAccountsSettings.tsx', { prefs, accounts: [principal, travail], busy: false, onUpdate: async () => true, onAdd: async () => true, onUntrack: async () => true, ...props }, [], 'CodexAccountsSettings');
  return component('../settings/ClaudeAccountsSettings.tsx', element.props, states);
};
// Settings useState order: tab, prefs, status, claude, error, busy, revision, accounts, accountsError, canChooseMode, accountActionError, codexAccounts, ...
const settings = (over = {}) => component('../settings/Settings.tsx', {}, ['providers', prefs, over.status ?? status, null, null, false, 0, [], null, true, null, [principal, travail], null, null]);

test('A: the + is the only control next to the list: no advanced chevron, no Actualiser, no relay button', () => {
  const html = renderToStaticMarkup(list());
  assert.doesNotMatch(html, /prefs-more|Chemins et options avancées/);
  assert.equal((html.match(/claude-accounts__add/g) || []).length, 1);
  const page = renderToStaticMarkup(settings());
  assert.doesNotMatch(page, /Actualiser|Retirer l’ancien relais|prefs-more/);
});

test('A: the + opens one small flow with the folder chooser AND the absolute path field, in place', () => {
  const closed = renderToStaticMarkup(list({ onChooseFolder: async () => null }));
  assert.doesNotMatch(closed, /Choisir un dossier|add-account__flow/);
  const open = renderToStaticMarkup(list({ onChooseFolder: async () => null }, ['', null, true, null]));
  assert.match(open, /<div[^>]*class="add-account__flow"[^>]*role="group"[^>]*aria-label="Ajouter un compte Codex"/);
  assert.match(open, />Choisir un dossier…<\/button>/);
  assert.match(open, /<input[^>]*aria-label="Chemin absolu d’un dossier CODEX_HOME"/);
  assert.equal((open.match(/role="dialog"|<dialog/g) || []).length, 0, 'no sub-window');
  // Without a native chooser (Windows/Linux): the path field alone.
  const bare = renderToStaticMarkup(list({}, ['', null, true, null]));
  assert.doesNotMatch(bare, /Choisir un dossier/);
  assert.match(bare, /<input[^>]*aria-label="Chemin absolu/);
});

test('A: the chooser and the typed path both add through the same handler, then close the flow', async () => {
  const added = [];
  const props = { onChooseFolder: async () => '/fixtures/.codex-choisi', onAdd: async (p) => { added.push(p); return true; } };
  const tree = list(props, ['/fixtures/.codex-saisi', null, true, null]);
  global.window = { requestAnimationFrame: fn => { fn(); return 1; } };
  const flow = find(tree, n => typeof n.props?.onChoose === 'function');
  await flow.props.onChoose();
  await flow.props.onSubmit();
  delete global.window;
  assert.deepEqual(added, ['/fixtures/.codex-choisi', '/fixtures/.codex-saisi']);
  assert.ok(env.changes.some(c => c.index === 2 && c.value === false), 'the flow closes after an add');
});

test('A: « Principal » is shown once per row (name with rename), the label only when the name differs', () => {
  const html = renderToStaticMarkup(list());
  const row = html.split('<li class="claude-accounts__row"')[1];
  assert.equal(text(row).filter(t => t === 'Principal').length, 1);
  assert.match(row, /aria-label="Renommer Principal"/);
  const renamed = renderToStaticMarkup(list({ accounts: [{ ...principal, name: 'Perso' }, travail] }));
  assert.match(renamed.split('<li class="claude-accounts__row"')[1], /<small>Principal<\/small>/);
});

test('A: the Codex connection (login, executable path) lives in the primary detail, only when connected', () => {
  const marker = 'EXTRA-CONNEXION';
  const open = (id) => renderToStaticMarkup(list({ detailExtra: (a) => a.primary ? marker : null }, ['', null, false, id]));
  assert.match(open('primary'), new RegExp(marker));
  assert.doesNotMatch(open(travail.id), new RegExp(marker));
  assert.doesNotMatch(open(null), new RegExp(marker));
  // Settings wires it: embedded connection for the primary when ready; none otherwise (the full panel is shown instead).
  const wired = (st) => find(settings({ status: st }), n => n.type?.name === 'CodexAccountsSettings').props.detailExtra;
  const extra = wired(status);
  assert.equal(typeof extra, 'function');
  const node = extra(principal);
  assert.equal(node.props.embedded, true);
  assert.equal(extra(travail), null);
  const off = wired({ ...status, phase: 'signed_out' });
  assert.ok(!off || off(principal) == null);
});

test('A: the embedded connection keeps login and executable path, without the intro text or the retry button', () => {
  const html = renderToStaticMarkup(component('../setup/CodexConnection.tsx', { status, onChange() {}, embedded: true }, [null, '', false]));
  assert.match(html, /Se connecter avec ChatGPT/);
  assert.match(html, /Connexion par code/);
  assert.match(html, /Emplacement de Codex/);
  assert.match(html, /Utiliser ce chemin/);
  assert.doesNotMatch(html, /Connexion directe à Codex|Les quotas sont lus automatiquement|Réessayer/);
  const full = renderToStaticMarkup(component('../setup/CodexConnection.tsx', { status, onChange() {} }, [null, '', false]));
  assert.match(full, /Connexion directe à Codex/);
});

// ---- Réglages focus from the native menu / compact window (volet C) ----------------------
const tick = () => new Promise(resolve => setImmediate(resolve));

test('C: an account list unfolds the detail of the account it is pointed at, or opens its add flow', () => {
  global.window = { requestAnimationFrame: fn => { fn(); return 1; } };
  const run = (focus) => {
    env.changes = [];
    list({ focus }, ['', null, false, null]);
    env.effects.forEach(effect => effect());
    return env.changes.map(c => [c.index, c.value]);
  };
  assert.deepEqual(run({ accountId: 'profile:Travail', add: false, nonce: 1 }), [[3, 'profile:Travail']]);
  assert.deepEqual(run({ accountId: null, add: true, nonce: 2 }), [[2, true]]);
  assert.deepEqual(run({ accountId: 'unknown', add: false, nonce: 3 }), [], 'an unknown account changes nothing');
  assert.deepEqual(run(null), []);
  delete global.window;
});

test('C: Settings hands the pending focus to the list of its provider only', () => {
  const focus = { provider: 'codex', accountId: 'profile:Travail', add: false, nonce: 7 };
  const tree = component('../settings/Settings.tsx', {}, ['providers', prefs, status, null, null, false, 0, [], null, true, null, [principal, travail], null, null, focus]);
  assert.deepEqual(find(tree, n => n.type?.name === 'CodexAccountsSettings').props.focus, focus);
  assert.equal(find(tree, n => n.type?.name === 'ClaudeAccountsSettings').props.focus ?? null, null);
});

test('C: Settings takes the pending focus when it opens and when it is woken, and lands on Fournisseurs', async () => {
  env.calls = []; env.changes = []; env.failure = null;
  env.result = (command) => command === 'take_settings_focus' ? { provider: 'claude', accountId: 'primary', add: false } : null;
  global.window = { setTimeout: () => 1, clearTimeout() {} };
  component('../settings/Settings.tsx', {}, ['notifications', prefs, status, null, null, false, 0, [], null, true, null, [principal], null, null]);
  env.effects.forEach(effect => effect());
  await tick(); await tick();
  delete global.window;
  assert.ok(env.calls.some(c => c.command === 'take_settings_focus'));
  assert.ok(env.changes.some(c => c.index === 0 && c.value === 'providers'), 'the Fournisseurs tab is selected');
  assert.ok(env.changes.some(c => c.index === 14 && c.value?.provider === 'claude' && c.value.accountId === 'primary' && typeof c.value.nonce === 'number'));
});
