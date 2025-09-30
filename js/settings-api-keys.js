// API Keys management for Gemini, Codex, and Claude

// Elements for API keys and modal
const addGeminiKeyBtn = document.getElementById('add-gemini-key-btn');
const addCodexKeyBtn = document.getElementById('add-codex-key-btn');
const addClaudeKeyBtn = document.getElementById('add-claude-key-btn');
const apiKeyModal = document.getElementById('api-key-modal');
const modalTitle = document.getElementById('modal-title');
const apiKeyForm = document.getElementById('api-key-form');
const apiKeyInput = document.getElementById('api-key-input');
const baseUrlInput = document.getElementById('base-url-input');
const baseUrlGroup = document.getElementById('base-url-group');
const apiKeyProxyUrlInput = document.getElementById('api-key-proxy-url-input');
const proxyUrlGroup = document.getElementById('proxy-url-group');
const modalClose = document.getElementById('modal-close');
const modalCancel = document.getElementById('modal-cancel');
const modalSave = document.getElementById('modal-save');

// State
let geminiKeys = [];
let codexKeys = [];
let claudeKeys = [];
let originalGeminiKeys = [];
let originalCodexKeys = [];
let originalClaudeKeys = [];
let currentApiType = null;
let currentEditIndex = null;

// Load all API keys
async function loadAllApiKeys() {
    try {
        await Promise.all([
            loadGeminiKeys(),
            loadCodexKeys(),
            loadClaudeKeys()
        ]);
    } catch (error) {
        console.error('Error loading API keys:', error);
        showError('Failed to load API keys');
    }
}

async function loadGeminiKeys() {
    try {
        geminiKeys = await configManager.getApiKeys('gemini');
        originalGeminiKeys = JSON.parse(JSON.stringify(geminiKeys));
        renderGeminiKeys();
    } catch (error) {
        console.error('Error loading Gemini keys:', error);
        showError('Failed to load Gemini API keys');
        renderGeminiKeys();
    }
}

async function loadCodexKeys() {
    try {
        codexKeys = await configManager.getApiKeys('codex');
        originalCodexKeys = JSON.parse(JSON.stringify(codexKeys));
        renderCodexKeys();
    } catch (error) {
        console.error('Error loading Codex keys:', error);
        showError('Failed to load Codex API keys');
        renderCodexKeys();
    }
}

async function loadClaudeKeys() {
    try {
        claudeKeys = await configManager.getApiKeys('claude');
        originalClaudeKeys = JSON.parse(JSON.stringify(claudeKeys));
        renderClaudeKeys();
    } catch (error) {
        console.error('Error loading Claude keys:', error);
        showError('Failed to load Claude API keys');
        renderClaudeKeys();
    }
}

function renderGeminiKeys() {
    const loading = document.getElementById('gemini-loading');
    const list = document.getElementById('gemini-keys-list');
    if (!list) return;
    if (loading) loading.style.display = 'none';
    if (geminiKeys.length === 0) {
        list.innerHTML = `
            <div class="empty-state">
                <div class="empty-state-icon">🔑</div>
                <div class="empty-state-text">No Gemini API Keys</div>
                <div class="empty-state-subtitle">Add your first Gemini API key to get started</div>
            </div>
        `;
        return;
    }
    list.innerHTML = '';
    geminiKeys.forEach((key, index) => {
        const keyItem = document.createElement('div');
        keyItem.className = 'api-key-item';
        keyItem.innerHTML = `
            <div class="api-key-info">
                <div class="api-key-value">${key}</div>
            </div>
            <div class="api-key-actions">
                <button class="api-key-btn edit" onclick="editGeminiKey(${index})">Edit</button>
                <button class="api-key-btn delete" onclick="deleteGeminiKey(${index})">Delete</button>
            </div>
        `;
        list.appendChild(keyItem);
    });
}

function renderCodexKeys() {
    const loading = document.getElementById('codex-loading');
    const list = document.getElementById('codex-keys-list');
    if (!list) return;
    if (loading) loading.style.display = 'none';
    if (codexKeys.length === 0) {
        list.innerHTML = `
            <div class="empty-state">
                <div class="empty-state-icon">🔑</div>
                <div class="empty-state-text">No Codex API Keys</div>
                <div class="empty-state-subtitle">Add your first Codex API key to get started</div>
            </div>
        `;
        return;
    }
    list.innerHTML = '';
    codexKeys.forEach((keyObj, index) => {
        const keyItem = document.createElement('div');
        keyItem.className = 'api-key-item';
        keyItem.innerHTML = `
            <div class="api-key-info">
                <div class="api-key-value">${keyObj['api-key']}</div>
                ${keyObj['base-url'] ? `<div class=\"api-key-base-url\">Base URL: ${keyObj['base-url']}</div>` : ''}
                ${keyObj['proxy-url'] ? `<div class=\"api-key-proxy-url\">Proxy URL: ${keyObj['proxy-url']}</div>` : ''}
            </div>
            <div class="api-key-actions">
                <button class="api-key-btn edit" onclick="editCodexKey(${index})">Edit</button>
                <button class="api-key-btn delete" onclick="deleteCodexKey(${index})">Delete</button>
            </div>
        `;
        list.appendChild(keyItem);
    });
}

function renderClaudeKeys() {
    const loading = document.getElementById('claude-loading');
    const list = document.getElementById('claude-keys-list');
    if (!list) return;
    if (loading) loading.style.display = 'none';
    if (claudeKeys.length === 0) {
        list.innerHTML = `
            <div class="empty-state">
                <div class="empty-state-icon">🔑</div>
                <div class="empty-state-text">No Claude API Keys</div>
                <div class="empty-state-subtitle">Add your first Claude API key to get started</div>
            </div>
        `;
        return;
    }
    list.innerHTML = '';
    claudeKeys.forEach((keyObj, index) => {
        const keyItem = document.createElement('div');
        keyItem.className = 'api-key-item';
        keyItem.innerHTML = `
            <div class="api-key-info">
                <div class="api-key-value">${keyObj['api-key']}</div>
                ${keyObj['base-url'] ? `<div class=\"api-key-base-url\">Base URL: ${keyObj['base-url']}</div>` : ''}
                ${keyObj['proxy-url'] ? `<div class=\"api-key-proxy-url\">Proxy URL: ${keyObj['proxy-url']}</div>` : ''}
            </div>
            <div class="api-key-actions">
                <button class="api-key-btn edit" onclick="editClaudeKey(${index})">Edit</button>
                <button class="api-key-btn delete" onclick="deleteClaudeKey(${index})">Delete</button>
            </div>
        `;
        list.appendChild(keyItem);
    });
}

// Modal: add/edit API key
function showApiKeyModal(type, editIndex = null) {
    currentApiType = type;
    currentEditIndex = editIndex;
    modalTitle.textContent = editIndex !== null ? 'Edit API Key' : 'Add API Key';
    apiKeyInput.value = '';
    baseUrlInput.value = '';
    apiKeyProxyUrlInput.value = '';
    apiKeyInput.classList.remove('error');
    baseUrlInput.classList.remove('error');
    apiKeyProxyUrlInput.classList.remove('error');

    if (type === 'codex' || type === 'claude') {
        baseUrlGroup.style.display = 'block';
        proxyUrlGroup.style.display = 'block';
    } else {
        baseUrlGroup.style.display = 'none';
        proxyUrlGroup.style.display = 'none';
    }

    if (editIndex !== null) {
        if (type === 'gemini') {
            apiKeyInput.value = geminiKeys[editIndex];
        } else if (type === 'codex') {
            const keyObj = codexKeys[editIndex];
            apiKeyInput.value = keyObj['api-key'] || '';
            baseUrlInput.value = keyObj['base-url'] || '';
            apiKeyProxyUrlInput.value = keyObj['proxy-url'] || '';
        } else if (type === 'claude') {
            const keyObj = claudeKeys[editIndex];
            apiKeyInput.value = keyObj['api-key'] || '';
            baseUrlInput.value = keyObj['base-url'] || '';
            apiKeyProxyUrlInput.value = keyObj['proxy-url'] || '';
        }
    }

    apiKeyModal.classList.add('show');
    apiKeyInput.focus();
}

function hideApiKeyModal() {
    apiKeyModal.classList.remove('show');
    currentApiType = null;
    currentEditIndex = null;
}

function saveApiKey() {
    const apiKey = apiKeyInput.value.trim();
    const baseUrl = baseUrlInput.value.trim();
    const proxyUrl = apiKeyProxyUrlInput.value.trim();
    apiKeyInput.classList.remove('error');
    baseUrlInput.classList.remove('error');
    apiKeyProxyUrlInput.classList.remove('error');

    const currentTab = document.querySelector('.tab.active').getAttribute('data-tab');
    if (currentTab !== 'api') {
        showError('Please switch to Third Party API Keys tab to manage keys');
        return;
    }

    let hasErrors = false;
    if (!apiKey) {
        apiKeyInput.classList.add('error');
        apiKeyInput.focus();
        showError('Please fill in this field');
        hasErrors = true;
    }

    if (!hasErrors && (currentApiType === 'codex' || currentApiType === 'claude')) {
        if (!baseUrl) {
            baseUrlInput.classList.add('error');
            baseUrlInput.focus();
            showError('Please fill in this field');
            hasErrors = true;
        }
    }

    if (hasErrors) return;

    if (currentApiType === 'gemini') {
        if (currentEditIndex !== null) {
            geminiKeys[currentEditIndex] = apiKey;
        } else {
            geminiKeys.push(apiKey);
        }
        renderGeminiKeys();
    } else if (currentApiType === 'codex') {
        const keyObj = { 'api-key': apiKey };
        if (baseUrl) keyObj['base-url'] = baseUrl;
        if (proxyUrl) keyObj['proxy-url'] = proxyUrl;
        if (currentEditIndex !== null) {
            codexKeys[currentEditIndex] = keyObj;
        } else {
            codexKeys.push(keyObj);
        }
        renderCodexKeys();
    } else if (currentApiType === 'claude') {
        const keyObj = { 'api-key': apiKey };
        if (baseUrl) keyObj['base-url'] = baseUrl;
        if (proxyUrl) keyObj['proxy-url'] = proxyUrl;
        if (currentEditIndex !== null) {
            claudeKeys[currentEditIndex] = keyObj;
        } else {
            claudeKeys.push(keyObj);
        }
        renderClaudeKeys();
    }

    hideApiKeyModal();
}

function editGeminiKey(index) { showApiKeyModal('gemini', index); }
function editCodexKey(index) { showApiKeyModal('codex', index); }
function editClaudeKey(index) { showApiKeyModal('claude', index); }

function deleteGeminiKey(index) {
    showConfirmDialog(
        'Confirm Delete',
        'Are you sure you want to delete this Gemini API key? This action cannot be undone.',
        () => {
            geminiKeys.splice(index, 1);
            renderGeminiKeys();
        }
    );
}

function deleteCodexKey(index) {
    showConfirmDialog(
        'Confirm Delete',
        'Are you sure you want to delete this Codex API key? This action cannot be undone.',
        () => {
            codexKeys.splice(index, 1);
            renderCodexKeys();
        }
    );
}

function deleteClaudeKey(index) {
    showConfirmDialog(
        'Confirm Delete',
        'Are you sure you want to delete this Claude API key? This action cannot be undone.',
        () => {
            claudeKeys.splice(index, 1);
            renderClaudeKeys();
        }
    );
}

// Modal events
modalClose.addEventListener('click', hideApiKeyModal);
modalCancel.addEventListener('click', hideApiKeyModal);
apiKeyForm.addEventListener('submit', (e) => { e.preventDefault(); saveApiKey(); });
modalSave.addEventListener('click', (e) => { e.preventDefault(); saveApiKey(); });
apiKeyModal.addEventListener('click', (e) => { if (e.target === apiKeyModal) hideApiKeyModal(); });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && apiKeyModal.classList.contains('show')) hideApiKeyModal(); });

// Clear inline errors on input
apiKeyInput.addEventListener('input', () => { if (apiKeyInput.classList.contains('error')) apiKeyInput.classList.remove('error'); });
baseUrlInput.addEventListener('input', () => { if (baseUrlInput.classList.contains('error')) baseUrlInput.classList.remove('error'); });
apiKeyProxyUrlInput.addEventListener('input', () => { if (apiKeyProxyUrlInput.classList.contains('error')) apiKeyProxyUrlInput.classList.remove('error'); });

// Buttons to open modal
addGeminiKeyBtn.addEventListener('click', () => showApiKeyModal('gemini'));
addCodexKeyBtn.addEventListener('click', () => showApiKeyModal('codex'));
addClaudeKeyBtn.addEventListener('click', () => showApiKeyModal('claude'));
