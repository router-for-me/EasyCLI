const vscode = require('vscode');
const fs = require('node:fs/promises');
const path = require('node:path');

// No network requests or direct access to VS Code's encrypted database. The
// native command encrypts the credential and returns a secret reference in JSON.
async function storeCredential(api, configUri, request) {
  if (api.workspace.getConfiguration('chat').get('disableAIFeatures') === true) throw new Error('ai-disabled');
  const copilot = api.extensions.getExtension('GitHub.copilot-chat');
  if (!copilot) throw new Error('copilot-disabled');
  await copilot.activate();
  const command = 'lm.addLanguageModelsProviderGroup';
  if (!(await api.commands.getCommands(true)).includes(command)) throw new Error('unsupported');
  const open = api.workspace.textDocuments.find(doc => doc.uri.toString() === configUri.toString());
  if (open?.isDirty) throw new Error('dirty');
  const name = `CPA credential ${request.id}`;
  const vendor = 'customendpoint';
  await api.commands.executeCommand(command, { vendor, name, apiKey: request.apiKey, models: [] });
  const document = await api.workspace.openTextDocument(configUri);
  const groups = JSON.parse(document.getText()); // Native command writes strict JSON.
  const group = groups.find(value => value.vendor === vendor && value.name === name);
  if (!/^\$\{input:chat\.lm\.secret\.[^}]+\}$/.test(group?.apiKey ?? '')) throw new Error('secret');
  const secretReference = group.apiKey;
  // Remove only our temporary group; ezcpa's existing transaction layer will
  // apply the actual model configuration with this reference and its backups.
  const edit = new api.WorkspaceEdit();
  edit.replace(configUri, new api.Range(document.positionAt(0), document.positionAt(document.getText().length)),
    JSON.stringify(groups.filter(value => value !== group), null, '\t'));
  if (!(await api.workspace.applyEdit(edit)) || !(await document.save())) throw new Error('save');
  return { secretReference, vendor };
}

async function activate(context) {
  // Never run or close a normal editor if somebody installs this package.
  if (context.extensionMode !== vscode.ExtensionMode.Development) return;
  const configUri = vscode.Uri.joinPath(context.globalStorageUri, '..', '..', 'chatLanguageModels.json');
  const directory = path.dirname(context.extensionUri.fsPath);
  const requestPath = path.join(directory, 'request.json');
  let timeout;
  let request;
  try {
    request = JSON.parse(await fs.readFile(requestPath, 'utf8'));
    // Remove the plaintext handoff immediately, including malformed/expired requests.
    await fs.unlink(requestPath);
    if (!/^[a-f0-9]{64}$/.test(request.id) || typeof request.apiKey !== 'string' || !request.apiKey
        || !Number.isSafeInteger(request.expiresAt) || request.expiresAt <= Date.now()
        || request.expiresAt > Date.now() + 180000) return;
    timeout = setTimeout(() => { void vscode.commands.executeCommand('workbench.action.closeWindow'); }, request.expiresAt - Date.now());
    let response;
    try {
      const expected = vscode.Uri.file(request.configPath);
      // VS Code exposes this local file through the vscode-userdata URI scheme.
      const normalize = value => process.platform === 'win32' ? value.toLowerCase() : value;
      if (normalize(expected.fsPath) !== normalize(configUri.fsPath)) throw new Error('profile-mismatch');
      response = { id: request.id, ...await storeCredential(vscode, configUri, request) };
    } catch (error) {
      // Never serialize command errors, which can interpolate credentials.
      const errors = ['copilot-disabled', 'ai-disabled', 'dirty', 'unsupported', 'profile-mismatch'];
      response = { id: request.id, error: errors.includes(error?.message) ? error.message : 'setup-failed' };
    }
    if (request.expiresAt <= Date.now()) return;
    // Atomic response avoids the parent reading a partially written result.
    const pending = path.join(directory, 'response.pending');
    await fs.writeFile(pending, JSON.stringify(response), { mode: 0o600 });
    await fs.rename(pending, path.join(directory, 'response.json'));
  } catch { /* Closed/cancelled run or unreadable request; parent reports timeout. */ }
  finally {
    if (request) request.apiKey = '';
    clearTimeout(timeout);
    await vscode.commands.executeCommand('workbench.action.closeWindow');
  }
}

module.exports = { activate, storeCredential };
