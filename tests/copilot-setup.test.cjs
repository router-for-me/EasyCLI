const { test } = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');

function loadHelper(api = {}, disk) {
  const extension = { exports: {} };
  const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/agents/copilot/setup-helper.cjs'), 'utf8');
  vm.runInNewContext(source, {
    require: name => name === 'vscode' ? api : name === 'node:fs/promises' && disk ? disk : require(name),
    module: extension, setTimeout, clearTimeout, process: { platform: process.platform },
  });
  return extension.exports;
}
const { storeCredential } = loadHelper();

function fixture({ disabled = false, dirty = false, supported = true } = {}) {
  const other = { vendor: 'openai', name: 'User provider', apiKey: '${input:user-key}' };
  let text = JSON.stringify([other]);
  const uri = { fsPath: '/profile/chatLanguageModels.json', toString: () => 'vscode-userdata:/profile/chatLanguageModels.json' };
  const calls = [];
  const document = { uri, isDirty: dirty, getText: () => text, positionAt: n => n, save: async () => true };
  const api = {
    extensions: { getExtension: () => disabled ? undefined : { activate: async () => {} } },
    commands: {
      getCommands: async () => supported ? ['lm.addLanguageModelsProviderGroup'] : [],
      executeCommand: async (command, value) => {
        calls.push({ command, value });
        assert.equal(value.vendor, 'customendpoint');
        assert.equal(value.apiKey, 'fixture-key');
        text = JSON.stringify([...JSON.parse(text), { ...value, apiKey: '${input:chat.lm.secret.native-fixture}' }]);
      },
    },
    workspace: {
      getConfiguration: () => ({ get: () => false }), textDocuments: [document],
      openTextDocument: async () => document,
      applyEdit: async edit => { text = edit.text; return true; },
    },
    WorkspaceEdit: class { replace(_uri, _range, value) { this.text = value; } },
    Range: class {},
  };
  return { api, uri, calls, other, result: () => JSON.parse(text) };
}

for (const scenario of ['success', 'expired', 'wrong-profile', 'installed']) {
  test(`one-time lifecycle: ${scenario}`, async () => {
    const f = fixture();
    const directory = path.resolve('fixture-setup');
    const requestPath = path.join(directory, 'request.json');
    const request = { id: 'a'.repeat(64), apiKey: 'fixture-key', configPath: '/profile/chatLanguageModels.json',
      expiresAt: Date.now() + (scenario === 'expired' ? -1 : 30000) };
    const files = new Map([[requestPath, JSON.stringify(request)]]);
    const disk = {
      readFile: async file => { if (!files.has(file)) throw new Error('missing'); return files.get(file); },
      unlink: async file => { files.delete(file); },
      writeFile: async (file, text) => { files.set(file, text); },
      rename: async (from, to) => { files.set(to, files.get(from)); files.delete(from); },
    };
    let closed = 0;
    const originalCommand = f.api.commands.executeCommand;
    f.api.commands.executeCommand = async (name, value) => name === 'workbench.action.closeWindow'
      ? ++closed : originalCommand(name, value);
    f.api.ExtensionMode = { Development: 2, Production: 1 };
    f.api.Uri = {
      joinPath: () => f.uri,
      file: () => scenario === 'wrong-profile' ? { fsPath: '/other/profile.json' } : { fsPath: '/profile/chatLanguageModels.json', toString: () => 'file:///profile/chatLanguageModels.json' },
    };
    await loadHelper(f.api, disk).activate({ extensionMode: scenario === 'installed' ? 1 : 2,
      extensionUri: { fsPath: path.join(directory, 'helper') }, globalStorageUri: {} });
    if (scenario === 'installed') {
      assert.equal(closed, 0);
      assert.ok(files.has(requestPath));
    } else {
      assert.equal(closed, 1);
      assert.ok(!files.has(requestPath));
      const response = files.get(path.join(directory, 'response.json'));
      if (scenario === 'success') {
        assert.equal(JSON.parse(response).secretReference, '${input:chat.lm.secret.native-fixture}');
        assert.ok(!response.includes('fixture-key'));
      } else if (scenario === 'wrong-profile') {
        assert.equal(JSON.parse(response).error, 'profile-mismatch');
      } else assert.equal(response, undefined);
    }
    assert.equal(f.calls.length, scenario === 'success' ? 1 : 0);
    assert.deepEqual(f.result(), [f.other]);
  });
}

test('native command receives the key; only the secret reference is returned and user groups survive', async () => {
  const f = fixture();
  const result = await storeCredential(f.api, f.uri, { id: 'fixture', apiKey: 'fixture-key' });
  assert.equal(result.secretReference, '${input:chat.lm.secret.native-fixture}');
  assert.equal(result.vendor, 'customendpoint');
  assert.deepEqual(f.result(), [f.other]);
  assert.equal(f.calls.length, 1);
  assert.ok(!JSON.stringify(result).includes('fixture-key'));
});

for (const [options, error] of [[{ disabled: true }, /copilot-disabled/], [{ dirty: true }, /dirty/], [{ supported: false }, /unsupported/]]) {
  test(`refuses unsafe or unavailable native setup: ${error.source}`, async () => {
    const f = fixture(options);
    await assert.rejects(storeCredential(f.api, f.uri, { id: 'fixture', apiKey: 'fixture-key' }), error);
    assert.equal(f.calls.length, 0);
    assert.deepEqual(f.result(), [f.other]);
  });
}
