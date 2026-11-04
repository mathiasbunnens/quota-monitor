// Dashboard (volet B) : en-tête par fournisseur, cartes de compte avec état + chevron,
// + par fournisseur. Même harnais que les autres tests de comptes.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { renderToStaticMarkup } = require('react-dom/server');
const { env, component, find } = require('./harness.cjs');

global.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
const NOW = Date.parse('2030-01-01T10:00:00Z');
const row = (period, remaining, windowMinutes, resetAt) => ({ model: 'codex', period, remaining, limit: 100, resetAt, checkedAt: '2030-01-01T09:59:00Z', source: 'codex', confidence: 'high', windowMinutes });
const rowsOf = (r5, r7) => [row('codex:codex:primary', r5, 300, '2030-01-01T12:00:00Z'), row('codex:codex:secondary', r7, 10080, '2030-01-04T12:00:00Z')];
const legacyRows = [row('five-hour', 63, 300, '2030-01-01T12:00:00Z'), row('weekly', 85, 10080, '2030-01-04T12:00:00Z')];
const codex = (name, extra = {}) => ({ id: name === 'Principal' ? 'primary' : 'profile:' + name, path: name === 'Principal' ? '~/.codex' : '/fixtures/.codex-' + name, name, visible: true, primary: name === 'Principal', tracked: true, available: true, refreshing: false, phase: 'ready', message: '', planType: 'plus', lastChecked: null, rows: rowsOf(63, 85), duplicateOf: null, availableResets: 2, ...extra });
const metric = (id, label, used) => ({ id, label, usedPercent: used, resetAt: '2030-01-01T12:00:00Z', availability: 'available', periodDurationSeconds: 18000, exhausted: false });
const claudeSnap = (extra = {}) => ({ provider: 'claude', schemaVersion: 1, enabled: true, configured: true, state: 'ready', source: null, observedAt: null, providerFetchedAt: null, message: '', errorKind: null, metrics: [metric('claude:five_hour', 'Limite 5 heures', 20), metric('claude:seven_day', 'Limite hebdomadaire', 40)], capability: { refresh: 'on_demand' }, planType: 'pro', extraUsage: null, ...extra });
const claudeAccount = (name, extra = {}) => ({ id: name === 'Principal' ? 'primary' : 'claude:' + name, path: '/fixtures/.claude-' + name, name, visible: true, primary: name === 'Principal', available: true, refreshing: false, snapshot: claudeSnap(), ...extra });
const status = { phase: 'ready', message: 'ok', executable: null, lastChecked: null, authUrl: null, userCode: null, planType: 'plus', activeInstances: null, refreshSeconds: 120, availableResets: 2, refreshSettings: { customSeconds: null } };
const prefs = { codexEnabled: true, claudeEnabled: true, fableEnabled: true, indicator: 'codex', displayMode: 'menubar', ntfy: { enabled: false, serverUrl: '', topic: '' }, layout: 'compact', claudeAccounts: { mode: 'multi', selectedId: null, profiles: [] }, codexAccounts: { mode: 'multi', selectedId: null, profiles: [] } };
// Dashboard useState order: snapshots, status, claude, prefs, showSetup, codexError, claudeError, error, refreshing, revision, accounts, now, codexAccounts.
const dashTree = (p, codexList, claudeList = []) => component('../../pages/Dashboard/Dashboard.tsx', {},
  [legacyRows, status, null, p, false, null, null, null, false, 0, claudeList, NOW, codexList]);
const dash = (...args) => renderToStaticMarkup(dashTree(...args));
const groups = (html) => html.split('<section class="provider-group').slice(1).map(g => g.slice(0, g.indexOf('</section></section>') > 0 ? g.indexOf('</section></section>') + 10 : undefined));
const cards = (html) => html.split('<section class="provider-card').slice(1);
const text = (html) => html.replace(/<[^>]+>/g, '|').split('|').map(s => s.trim()).filter(Boolean);

const tracked = [codex('Principal'), codex('Travail', { planType: 'pro' }), codex('Perso', { phase: 'signed_out', planType: null, rows: [], availableResets: null }), codex('Alias', { duplicateOf: 'Principal' })];
const claudes = [claudeAccount('Principal'), claudeAccount('Travail', { snapshot: claudeSnap({ planType: 'max' }) })];

test('B: one header per provider, « Codex ↗ » / « Claude ↗ », before its cards (not on the first card only)', () => {
  const html = dash(prefs, tracked, claudes);
  const list = groups(html);
  assert.equal(list.length, 2);
  assert.match(list[0], /^[^>]*aria-labelledby="codex-group"[\s\S]*<h2 id="codex-group"[^>]*><button[^>]*class="provider-card__usage"[^>]*aria-label="Codex : usage détaillé sur chatgpt\.com"[^>]*>Codex <svg class="external-icon"/);
  assert.match(list[1], /<h2 id="claude-group"[^>]*><button[^>]*aria-label="Claude : usage détaillé sur claude\.ai"[^>]*>Claude <svg class="external-icon"/);
  assert.equal((html.match(/class="provider-card__usage"/g) || []).length, 2, 'exactly once per provider');
  assert.equal((list[0].match(/<section class="provider-card/g) || []).length, 4);
  assert.equal((list[1].match(/<section class="provider-card/g) || []).length, 2);
  assert.ok(list[0].indexOf('provider-card__usage') < list[0].indexOf('<section class="provider-card'), 'header first');
  assert.doesNotMatch(html, /<h[23][^>]*>[^<]*(Codex|Claude) · /, 'no « Codex · nom » card title');
});

test('B: the header opens the provider usage page', async () => {
  env.calls = []; env.result = null; env.failure = null;
  const tree = dashTree(prefs, tracked, claudes);
  const buttons = [];
  const walk = (n) => { if (!n || typeof n !== 'object') return; if (n.props?.usage) buttons.push(n.props.usage); [].concat(n.props?.children ?? []).forEach(walk); };
  walk(tree);
  assert.equal(buttons.length, 2);
  buttons.forEach(u => u.onOpen());
  await new Promise(r => setImmediate(r));
  assert.deepEqual(env.calls.map(c => c.command), ['open_usage_details', 'open_claude_usage']);
});

test('B: the card shows name, plan badge beside the title, reset, state, then the detail chevron', () => {
  const html = dash(prefs, tracked, claudes);
  const first = cards(html)[0];
  const title = first.match(/<h3 class="provider-card__title">([\s\S]*?)<\/h3>/)[1];
  assert.match(title, /^<span class="provider-card__name">Principal<\/span><span class="plan-badge">plus<\/span>$/, 'name and badge in one title line');
  const order = ['provider-card__title', 'reset-chip', 'provider-card__pill', 'aria-label="Détail Codex Principal"'].map(t => first.indexOf(t));
  assert.ok(order.every(i => i > 0) && order.every((v, i) => i === 0 || v > order[i - 1]), 'title, reset, state, chevron: ' + order);
  assert.match(first, /provider-card__pill provider-card__pill--good">Connecté</);
  assert.doesNotMatch(html, />Détails</, 'the Détails button is replaced by the chevron');
  const list = cards(html);
  assert.match(list[2], /provider-card__pill--warning"[^>]*>Non connecté</);
  assert.match(list[3], /provider-card__pill--warning"[^>]*>Même compte</);
  assert.doesNotMatch(list[2], /reset-chip/);
  assert.match(cards(html)[4], /aria-label="Quotas Claude · Principal"/);
  assert.match(cards(html)[5], /aria-label="Détail Claude Travail"/);
});

test('B: the chevron opens the detail of THAT account only (its folder), and toggles', () => {
  const props = { name: 'Travail', provider: 'Codex', badge: 'pro', rows: [], compact: false, now: NOW, state: { label: 'Connecté', tone: 'good', detail: '' }, detail: { path: '/fixtures/.codex-Travail', label: 'Détail Codex Travail' } };
  const closed = renderToStaticMarkup(component('../quota/ProviderCard.tsx', props, []));
  assert.match(closed, /<button[^>]*aria-expanded="false"[^>]*aria-label="Détail Codex Travail"/);
  assert.doesNotMatch(closed, /\.codex-Travail/);
  // ProviderCard state order: hover, pinned, focused.
  const open = renderToStaticMarkup(component('../quota/ProviderCard.tsx', props, [false, true, false]));
  assert.match(open, /aria-expanded="true"/);
  assert.match(open, /<code[^>]*>\/fixtures\/\.codex-Travail<\/code>/);
  const tree = component('../quota/ProviderCard.tsx', props, []);
  const chevron = find(tree, n => n.props?.['aria-label'] === 'Détail Codex Travail');
  env.changes = []; chevron.props.onClick();
  assert.ok(env.changes.some(c => c.index === 1), 'toggles the pinned state of this card');
  // Two accounts: each region shows its own folder.
  const html = dash(prefs, tracked, claudes);
  assert.equal((html.match(/aria-label="Détail (Codex |Claude )?[^"]+"/g) || []).length, 6);
});

test('B: the + at the bottom right of each provider names it, and sits after its cards', () => {
  const html = dash(prefs, tracked, claudes);
  const [a, b] = groups(html);
  assert.equal((a.match(/aria-label="Ajouter un compte Codex"/g) || []).length, 1);
  assert.equal((b.match(/aria-label="Ajouter un compte Claude"/g) || []).length, 1);
  assert.doesNotMatch(a, /Ajouter un compte Claude/);
  assert.ok(a.lastIndexOf('<section class="provider-card') < a.indexOf('Ajouter un compte Codex'), 'after the last card');
  assert.match(a, /class="provider-group__footer"/);
});

test('B: the + routes its folder chooser and typed path to its own provider', async () => {
  const run = async (provider, chosen) => {
    env.calls = []; env.failure = null;
    env.result = (command) => command === 'get_platform' ? 'macos' : command.startsWith('choose_') ? chosen : null;
    // AddAccountButton state order: open, path, error, busy, canChoose.
    const flow = find(component('../accounts/AddAccountButton.tsx', { provider, onChange() {} }, [true, '/fixtures/.x-saisi', null, false, true]), n => typeof n.props?.onChoose === 'function');
    global.window = { requestAnimationFrame: fn => { fn(); return 1; } };
    await flow.props.onChoose();
    await flow.props.onSubmit();
    delete global.window;
    return env.calls.map(c => [c.command, c.payload]);
  };
  assert.deepEqual(await run('Codex', '/fixtures/.codex-choisi'), [['choose_codex_profile_folder', undefined], ['add_codex_profile', { path: '/fixtures/.codex-choisi' }], ['add_codex_profile', { path: '/fixtures/.x-saisi' }]]);
  assert.deepEqual(await run('Claude', '/fixtures/.claude-choisi'), [['choose_claude_profile_folder', undefined], ['add_claude_profile', { path: '/fixtures/.claude-choisi' }], ['add_claude_profile', { path: '/fixtures/.x-saisi' }]]);
  assert.deepEqual(await run('Claude', null), [['choose_claude_profile_folder', undefined], ['add_claude_profile', { path: '/fixtures/.x-saisi' }]]);
});

test('B: Compacte and Classique both keep the provider structure; Classique has no paired bars', () => {
  for (const layout of ['compact', 'classic']) {
    const html = dash({ ...prefs, layout }, tracked.slice(0, 2), claudes);
    assert.equal(groups(html).length, 2, layout);
    assert.equal((html.match(/class="provider-card__usage"/g) || []).length, 2, layout);
    assert.equal((html.match(/provider-card__paired-bars/g) || []).length > 0, layout === 'compact', layout);
    assert.equal((html.match(/aria-label="Ajouter un compte /g) || []).length, 2, layout);
  }
});

test('B: a single account per provider keeps header, card, state and + (no « Codex · » prefix)', () => {
  const html = dash({ ...prefs, codexAccounts: { mode: 'single', selectedId: null, profiles: [] }, claudeAccounts: { mode: 'single', selectedId: null, profiles: [] } }, [tracked[0]], [claudes[0]]);
  assert.equal(groups(html).length, 2);
  assert.deepEqual(text('<section class="provider-card' + cards(html)[0].replace(/^[^>]*>/, '>')).slice(0, 2), ['Principal', 'plus']);
  assert.match(html, /aria-label="Détail Claude Principal"/);
});

test('B: a disabled provider has no group, header or +', () => {
  const html = dash({ ...prefs, claudeEnabled: false }, tracked.slice(0, 2), []);
  assert.equal(groups(html).length, 1);
  assert.doesNotMatch(html, /Claude/);
});
