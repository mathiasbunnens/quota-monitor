// Shared static-render harness for the account views (hooks stubbed, IPC recorded).
// Same approach as codexAccounts.test.cjs; not a test file itself.
const fs = require('node:fs');
const Module = require('node:module');
const path = require('node:path');
const ts = require('typescript');
const React = require('react');

const env = { hooks: null, calls: [], result: null, failure: null, effects: [], changes: [] };
const load = Module._load;
Module._load = function (name, ...args) {
  if (name === 'react' && env.hooks) return { ...React, ...env.hooks };
  if (name === '@tauri-apps/api/core') return { invoke: async (command, payload) => {
    env.calls.push({ command, payload });
    if (env.failure) throw env.failure;
    return typeof env.result === 'function' ? env.result(command, payload) : env.result;
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

/** Calls a component function with stubbed hooks; `states` feeds useState by call order. */
function component(file, props, states = [], exportName = 'default') {
  let cursor = 0;
  env.effects = [];
  env.hooks = {
    useState: initial => {
      const index = cursor++;
      return [index < states.length ? states[index] : typeof initial === 'function' ? initial() : initial,
        value => env.changes.push({ index, value })];
    },
    useRef: initial => ({ current: initial }), useId: () => 'fixture-id', useEffect: effect => env.effects.push(effect),
    useLayoutEffect: () => {}, useCallback: fn => fn,
  };
  const filename = require.resolve(path.resolve(__dirname, file));
  for (const cached of Object.keys(require.cache)) {
    if (cached.startsWith(path.resolve(__dirname, '../../..') + path.sep) && !cached.includes('node_modules') && !cached.endsWith('.cjs')) delete require.cache[cached];
  }
  try { return require(filename)[exportName](props); } finally { env.hooks = null; }
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
module.exports = { env, component, find, React };
