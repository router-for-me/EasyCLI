// Core settings: element refs, initialization, and apply/reset logic
// Comments in English per project guidelines; embedded UI text remains original language when applicable.

// Debug and basic settings elements
const debugSwitch = document.getElementById('debug-switch');
const proxyUrlInput = document.getElementById('proxy-url-input');
const portInput = document.getElementById('port-input');
const requestLogSwitch = document.getElementById('request-log-switch');
const requestRetryInput = document.getElementById('request-retry-input');
const allowRemoteSwitch = document.getElementById('allow-remote-switch');
const secretKeyInput = document.getElementById('secret-key-input');
const switchProjectSwitch = document.getElementById('switch-project-switch');
const switchPreviewModelSwitch = document.getElementById('switch-preview-model-switch');
const autoStartSwitch = document.getElementById('auto-start-switch');

// Action buttons
const applyBtn = document.getElementById('apply-btn');
const resetBtn = document.getElementById('reset-btn');

// Server status display
const serverStatusText = document.getElementById('server-status-text');

// Store original config for comparison
let originalConfig = null;

// Initialize debug switch from config
async function initializeDebugSwitch() {
    try {
        const config = await configManager.getConfig();
        debugSwitch.checked = config.debug || false;
    } catch (error) {
        console.error('Error loading config:', error);
        debugSwitch.checked = false;
    }
}

// Initialize proxy URL from config
async function initializeProxyUrl() {
    try {
        const config = await configManager.getConfig();
        proxyUrlInput.value = config['proxy-url'] || '';
    } catch (error) {
        console.error('Error loading config:', error);
        proxyUrlInput.value = '';
    }
}

// Initialize port from config
async function initializePort() {
    try {
        const config = await configManager.getConfig();
        portInput.value = config.port || 8080;
    } catch (error) {
        console.error('Error loading config:', error);
        portInput.value = 8080;
    }
}

// Initialize remote management settings from config
async function initializeRemoteManagement() {
    try {
        const config = await configManager.getConfig();
        const remoteManagement = config['remote-management'] || {};
        allowRemoteSwitch.checked = remoteManagement['allow-remote'] || false;
        // Do not prefill the secret key for security
        secretKeyInput.value = '';
    } catch (error) {
        console.error('Error loading config:', error);
        allowRemoteSwitch.checked = false;
        secretKeyInput.value = '';
    }
}

// Show/hide Local mode specific fields
function toggleLocalOnlyFields() {
    const connectionType = localStorage.getItem('type') || 'local';
    const localOnlyFields = document.querySelectorAll('.local-only');
    const remoteOnlyFields = document.querySelectorAll('.remote-only');

    if (connectionType === 'local') {
        localOnlyFields.forEach(field => field.classList.add('show'));
        remoteOnlyFields.forEach(field => field.classList.remove('show'));
    } else {
        localOnlyFields.forEach(field => field.classList.remove('show'));
        remoteOnlyFields.forEach(field => field.classList.add('show'));
    }

    // Update server status when connection type changes
    updateServerStatus();
}

// Update server status display
function updateServerStatus() {
    const connectionType = localStorage.getItem('type') || 'local';

    if (connectionType === 'local') {
        serverStatusText.innerHTML = '<span style="color: #10b981;">●</span> Local';
    } else {
        configManager.refreshConnection();
        const storedBaseUrl = localStorage.getItem('base-url');
        const baseUrl = storedBaseUrl || configManager.baseUrl;

        if (baseUrl) {
            serverStatusText.innerHTML = `Remote:<br><span style="color: #10b981;">●</span> ${baseUrl}`;
        } else {
            serverStatusText.innerHTML = 'Remote:<br><span style="color: #10b981;">●</span> Unknown';
        }
    }
}

window.addEventListener('storage', (event) => {
    if (!event || event.key === null || event.key === 'base-url' || event.key === 'type') {
        updateServerStatus();
    }
});


// Initialize additional settings from config
async function initializeAdditionalSettings() {
    try {
        const config = await configManager.getConfig();
        requestLogSwitch.checked = config['request-log'] || false;
        requestRetryInput.value = config['request-retry'] ?? 3;

        if (config['quota-exceeded']) {
            switchProjectSwitch.checked = config['quota-exceeded']['switch-project'] || false;
            switchPreviewModelSwitch.checked = config['quota-exceeded']['switch-preview-model'] || false;
        } else {
            switchProjectSwitch.checked = false;
            switchPreviewModelSwitch.checked = false;
        }
    } catch (error) {
        console.error('Error loading config:', error);
        requestLogSwitch.checked = false;
        requestRetryInput.value = 3;
        switchProjectSwitch.checked = false;
        switchPreviewModelSwitch.checked = false;
    }
}

// Initialize auto-start switch
async function initializeAutoStart() {
    try {
        if (window.__TAURI__?.core?.invoke) {
            const result = await window.__TAURI__.core.invoke('check_auto_start_enabled');
            autoStartSwitch.checked = result.enabled || false;
        }
    } catch (error) {
        console.error('Error checking auto-start status:', error);
        autoStartSwitch.checked = false;
    }
}

// Handle auto-start toggle change
autoStartSwitch.addEventListener('change', async () => {
    try {
        if (window.__TAURI__?.core?.invoke) {
            if (autoStartSwitch.checked) {
                const result = await window.__TAURI__.core.invoke('enable_auto_start');
                if (result.success) {
                    showSuccessMessage('Auto-start enabled successfully');
                } else {
                    showError('Failed to enable auto-start');
                    autoStartSwitch.checked = false;
                }
            } else {
                const result = await window.__TAURI__.core.invoke('disable_auto_start');
                if (result.success) {
                    showSuccessMessage('Auto-start disabled successfully');
                } else {
                    showError('Failed to disable auto-start');
                    autoStartSwitch.checked = true;
                }
            }
        }
    } catch (error) {
        console.error('Error toggling auto-start:', error);
        showError('Failed to update auto-start setting');
        // Revert the toggle
        autoStartSwitch.checked = !autoStartSwitch.checked;
    }
});

// Get current config from server
async function getCurrentConfig() {
    try {
        return await configManager.getConfig();
    } catch (error) {
        console.error('Error getting current config:', error);
        throw error;
    }
}


// Update a single setting via abstraction layer
async function updateSetting(endpoint, value, isDelete = false) {
    try {
        return await configManager.updateSetting(endpoint, value, isDelete);
    } catch (error) {
        console.error(`Error updating ${endpoint}:`, error);
        return false;
    }
}

// Apply settings for the current tab
async function applyAllSettings() {
    applyBtn.disabled = true;
    applyBtn.textContent = 'Applying...';

    try {
        const serverConfig = await getCurrentConfig();
        const currentTab = document.querySelector('.tab.active').getAttribute('data-tab');
        const changes = [];

        if (currentTab === 'basic') {
            if (debugSwitch.checked !== (serverConfig.debug || false)) {
                changes.push({ endpoint: 'debug', value: debugSwitch.checked });
            }

            const connectionType = localStorage.getItem('type') || 'local';
            if (connectionType === 'local') {
                const serverPort = serverConfig.port || 8080;
                if (parseInt(portInput.value) !== serverPort) {
                    changes.push({ endpoint: 'port', value: parseInt(portInput.value) });
                }
            }

            const serverProxyUrl = serverConfig['proxy-url'] || '';
            if (proxyUrlInput.value !== serverProxyUrl) {
                if (proxyUrlInput.value.trim() === '') {
                    changes.push({ endpoint: 'proxy-url', value: '', isDelete: true });
                } else {
                    changes.push({ endpoint: 'proxy-url', value: proxyUrlInput.value });
                }
            }

            if (requestLogSwitch.checked !== (serverConfig['request-log'] || false)) {
                changes.push({ endpoint: 'request-log', value: requestLogSwitch.checked });
            }

            const serverRetry = serverConfig['request-retry'] ?? 3;
            if (parseInt(requestRetryInput.value) !== serverRetry) {
                changes.push({ endpoint: 'request-retry', value: parseInt(requestRetryInput.value) });
            }


            if (connectionType === 'local') {
                const serverRemoteManagement = serverConfig['remote-management'] || {};
                if (allowRemoteSwitch.checked !== (serverRemoteManagement['allow-remote'] || false)) {
                    changes.push({ endpoint: 'remote-management.allow-remote', value: allowRemoteSwitch.checked });
                }

                if (secretKeyInput.value.trim() !== '') {
                    changes.push({ endpoint: 'remote-management.secret-key', value: secretKeyInput.value });
                }
            }

            const serverQuotaExceeded = serverConfig['quota-exceeded'] || {};
            if (switchProjectSwitch.checked !== (serverQuotaExceeded['switch-project'] || false)) {
                changes.push({ endpoint: 'quota-exceeded/switch-project', value: switchProjectSwitch.checked });
            }
            if (switchPreviewModelSwitch.checked !== (serverQuotaExceeded['switch-preview-model'] || false)) {
                changes.push({ endpoint: 'quota-exceeded/switch-preview-model', value: switchPreviewModelSwitch.checked });
            }
        } else if (currentTab === 'access-token') {
            let serverApiKeys = serverConfig['api-keys'] || [];
            if (serverApiKeys === null) serverApiKeys = [];
            if (JSON.stringify(accessTokenKeys) !== JSON.stringify(serverApiKeys)) {
                changes.push({ endpoint: 'api-keys', value: accessTokenKeys });
            }
        } else if (currentTab === 'api') {
            let serverGeminiKeys = serverConfig['generative-language-api-key'] || [];
            if (serverGeminiKeys === null) serverGeminiKeys = [];
            if (JSON.stringify(geminiKeys) !== JSON.stringify(serverGeminiKeys)) {
                changes.push({ endpoint: 'generative-language-api-key', value: geminiKeys });
            }

            let serverCodexKeys = serverConfig['codex-api-key'] || [];
            if (serverCodexKeys === null) serverCodexKeys = [];
            if (JSON.stringify(codexKeys) !== JSON.stringify(serverCodexKeys)) {
                changes.push({ endpoint: 'codex-api-key', value: codexKeys });
            }

            let serverClaudeKeys = serverConfig['claude-api-key'] || [];
            if (serverClaudeKeys === null) serverClaudeKeys = [];
            if (JSON.stringify(claudeKeys) !== JSON.stringify(serverClaudeKeys)) {
                changes.push({ endpoint: 'claude-api-key', value: claudeKeys });
            }
        } else if (currentTab === 'openai') {
            let serverOpenaiProviders = serverConfig['openai-compatibility'] || [];
            if (serverOpenaiProviders === null) serverOpenaiProviders = [];
            if (JSON.stringify(openaiProviders) !== JSON.stringify(serverOpenaiProviders)) {
                changes.push({ endpoint: 'openai-compatibility', value: openaiProviders });
            }
        }

        let successCount = 0;
        let portChanged = false;
        for (const change of changes) {
            const ok = await updateSetting(change.endpoint, change.value, change.isDelete || false);
            if (ok) {
                successCount++;
                if (change.endpoint === 'port') {
                    portChanged = true;
                }
            }
        }

        if (changes.length === 0) {
            const tabName = currentTab === 'basic' ? 'Basic Settings' :
                currentTab === 'api' ? 'Third Party API Keys' : 'Settings';
            showSuccessMessage(`No changes to apply in ${tabName.toLowerCase()}`);
        } else if (successCount === changes.length) {
            const updatedConfig = await getCurrentConfig();
            originalConfig = updatedConfig;

            if (currentTab === 'basic') {
                await initializeDebugSwitch();
                await initializePort();
                await initializeProxyUrl();
                await initializeRemoteManagement();
                await initializeAdditionalSettings();
                toggleLocalOnlyFields();
            } else if (currentTab === 'access-token') {
                await loadAccessTokenKeys();
            } else if (currentTab === 'api') {
                await loadAllApiKeys();
            } else if (currentTab === 'openai') {
                await loadOpenaiProviders();
            }

            const tabName = currentTab === 'basic' ? 'Basic Settings' :
                currentTab === 'access-token' ? 'Access Token' :
                    currentTab === 'api' ? 'Third Party API Keys' :
                        currentTab === 'openai' ? 'OpenAI Compatibility' : 'Settings';
            showSuccessMessage(`Applied ${successCount} ${tabName.toLowerCase()} setting(s) successfully`);

            if (portChanged && localStorage.getItem('type') === 'local') {
                console.log('Port configuration has changed, need to restart CLIProxyAPI process');
                showSuccessMessage('Port configuration saved, restarting CLIProxyAPI process...');
                if (window.__TAURI__?.core?.invoke) {
                    window.__TAURI__.core.invoke('restart_cliproxyapi');
                }
            }
        } else {
            showError(`Failed to apply ${changes.length - successCount} setting(s)`);
        }
    } catch (error) {
        console.error('Error applying settings:', error);
        showError('Network error');
    } finally {
        applyBtn.disabled = false;
        applyBtn.textContent = 'Apply';
    }
}

// Reset settings to server values for the current tab
async function resetAllSettings() {
    try {
        const serverConfig = await getCurrentConfig();
        const currentTab = document.querySelector('.tab.active').getAttribute('data-tab');

        if (currentTab === 'basic') {
            debugSwitch.checked = serverConfig.debug || false;
            proxyUrlInput.value = serverConfig['proxy-url'] || '';
            requestLogSwitch.checked = serverConfig['request-log'] || false;
            requestRetryInput.value = serverConfig['request-retry'] ?? 3;

            const connectionType = localStorage.getItem('type') || 'local';
            if (connectionType === 'local') {
                portInput.value = serverConfig.port || 8080;
                const serverRemoteManagement = serverConfig['remote-management'] || {};
                allowRemoteSwitch.checked = serverRemoteManagement['allow-remote'] || false;
                secretKeyInput.value = '';
            }

            const serverQuotaExceeded = serverConfig['quota-exceeded'] || {};
            switchProjectSwitch.checked = serverQuotaExceeded['switch-project'] || false;
            switchPreviewModelSwitch.checked = serverQuotaExceeded['switch-preview-model'] || false;
        } else if (currentTab === 'access-token') {
            let serverApiKeys = serverConfig['api-keys'] || [];
            if (serverApiKeys === null) serverApiKeys = [];
            accessTokenKeys = JSON.parse(JSON.stringify(serverApiKeys));
            originalAccessTokenKeys = JSON.parse(JSON.stringify(accessTokenKeys));
            renderAccessTokenKeys();
        } else if (currentTab === 'api') {
            let serverGeminiKeys = serverConfig['generative-language-api-key'] || [];
            let serverCodexKeys = serverConfig['codex-api-key'] || [];
            let serverClaudeKeys = serverConfig['claude-api-key'] || [];
            if (serverGeminiKeys === null) serverGeminiKeys = [];
            if (serverCodexKeys === null) serverCodexKeys = [];
            if (serverClaudeKeys === null) serverClaudeKeys = [];
            geminiKeys = JSON.parse(JSON.stringify(serverGeminiKeys));
            codexKeys = JSON.parse(JSON.stringify(serverCodexKeys));
            claudeKeys = JSON.parse(JSON.stringify(serverClaudeKeys));
            originalGeminiKeys = JSON.parse(JSON.stringify(geminiKeys));
            originalCodexKeys = JSON.parse(JSON.stringify(codexKeys));
            originalClaudeKeys = JSON.parse(JSON.stringify(claudeKeys));
            renderGeminiKeys();
            renderCodexKeys();
            renderClaudeKeys();
        } else if (currentTab === 'openai') {
            let serverOpenaiProviders = serverConfig['openai-compatibility'] || [];
            if (serverOpenaiProviders === null) serverOpenaiProviders = [];
            openaiProviders = JSON.parse(JSON.stringify(serverOpenaiProviders));
            originalOpenaiProviders = JSON.parse(JSON.stringify(openaiProviders));
            renderOpenaiProviders();
        }

        originalConfig = serverConfig;
        const tabName = currentTab === 'basic' ? 'Basic Settings' :
            currentTab === 'access-token' ? 'Access Token' :
                currentTab === 'api' ? 'Third Party API Keys' :
                    currentTab === 'openai' ? 'OpenAI Compatibility' : 'Settings';
        showSuccessMessage(`${tabName} reset to server config`);
    } catch (error) {
        console.error('Error resetting settings:', error);
        showError('Failed to reset settings');
    }
}

// Wire core button events
applyBtn.addEventListener('click', applyAllSettings);
resetBtn.addEventListener('click', resetAllSettings);
