// OpenAI Compatibility providers management + dynamic inputs helpers

// Elements
const addProviderBtn = document.getElementById('add-provider-btn');
const openaiProviderModal = document.getElementById('openai-provider-modal');
const providerModalTitle = document.getElementById('provider-modal-title');
const openaiProviderForm = document.getElementById('openai-provider-form');
const providerNameInput = document.getElementById('provider-name-input');
const providerBaseUrlInput = document.getElementById('provider-base-url-input');
const apiKeysContainer = document.getElementById('api-keys-container');
const modelsContainer = document.getElementById('models-container');
const providerModalClose = document.getElementById('provider-modal-close');
const providerModalCancel = document.getElementById('provider-modal-cancel');
const providerModalSave = document.getElementById('provider-modal-save');

// State
let openaiProviders = [];
let originalOpenaiProviders = [];
let currentProviderEditIndex = null;

// Load providers
async function loadOpenaiProviders() {
    try {
        openaiProviders = await configManager.getApiKeys('openai');
        originalOpenaiProviders = JSON.parse(JSON.stringify(openaiProviders));
        renderOpenaiProviders();
    } catch (error) {
        console.error('Error loading OpenAI providers:', error);
        showError('Failed to load OpenAI providers');
        renderOpenaiProviders();
    }
}

function renderOpenaiProviders() {
    const loading = document.getElementById('openai-loading');
    const list = document.getElementById('openai-providers-list');
    if (!list) return;
    if (loading) loading.style.display = 'none';
    if (openaiProviders.length === 0) {
        list.innerHTML = `
            <div class="empty-state">
                <div class="empty-state-icon">🤖</div>
                <div class="empty-state-text">No OpenAI Compatibility Providers</div>
                <div class="empty-state-subtitle">Add your first provider to get started</div>
            </div>
        `;
        return;
    }
    list.innerHTML = '';
    openaiProviders.forEach((provider, index) => {
        const providerItem = document.createElement('div');
        providerItem.className = 'openai-provider-item';
        const apiKeysCount = provider['api-key-entries'] ? provider['api-key-entries'].length : 0;
        const modelsCount = provider.models ? provider.models.length : 0;
        providerItem.innerHTML = `
            <div class="openai-provider-info">
                <div class="openai-provider-name">${provider.name}</div>
                <div class="openai-provider-base-url">${provider['base-url']}</div>
                <div class="openai-provider-details">
                    <div class="openai-provider-detail">
                        <strong>${apiKeysCount}</strong> API Key${apiKeysCount !== 1 ? 's' : ''}
                    </div>
                    <div class="openai-provider-detail">
                        <strong>${modelsCount}</strong> Model${modelsCount !== 1 ? 's' : ''}
                    </div>
                </div>
            </div>
            <div class="openai-provider-actions">
                <button class="openai-provider-btn edit" onclick="editOpenaiProvider(${index})">Edit</button>
                <button class="openai-provider-btn delete" onclick="deleteOpenaiProvider(${index})">Delete</button>
            </div>
        `;
        list.appendChild(providerItem);
    });
}

function showOpenaiProviderModal(editIndex = null) {
    currentProviderEditIndex = editIndex;
    providerModalTitle.textContent = editIndex !== null ? 'Edit Provider' : 'Add Provider';
    providerNameInput.value = '';
    providerBaseUrlInput.value = '';
    clearProviderFormErrors();
    clearDynamicInputs();
    if (editIndex !== null) {
        const provider = openaiProviders[editIndex];
        providerNameInput.value = provider.name || '';
        providerBaseUrlInput.value = provider['base-url'] || '';
        const apiKeys = provider['api-key-entries'] || [];
        const models = provider.models || [];
        populateDynamicInputs(apiKeys, models);
    }
    openaiProviderModal.classList.add('show');
    providerNameInput.focus();
}

function hideOpenaiProviderModal() {
    openaiProviderModal.classList.remove('show');
    currentProviderEditIndex = null;
}

function saveOpenaiProvider() {
    const name = providerNameInput.value.trim();
    const baseUrl = providerBaseUrlInput.value.trim();
    const currentTab = document.querySelector('.tab.active').getAttribute('data-tab');
    if (currentTab !== 'openai') {
        showError('Please switch to OpenAI Compatibility tab to manage providers');
        return;
    }
    clearProviderFormErrors();
    let hasErrors = false;
    if (!name) {
        showProviderFieldError(providerNameInput, 'Please fill in this field');
        hasErrors = true;
    }
    if (!baseUrl) {
        showProviderFieldError(providerBaseUrlInput, 'Please fill in this field');
        hasErrors = true;
    }
    const { apiKeys, models } = getDynamicInputData();
    if (apiKeys.length === 0) {
        const firstApiKeyInput = apiKeysContainer.querySelector('.dynamic-input:first-child');
        if (firstApiKeyInput) showProviderFieldError(firstApiKeyInput, 'Please fill in this field');
        hasErrors = true;
    }
    if (!hasErrors && apiKeys.length > 0) {
        for (let i = apiKeys.length - 1; i >= 0; i--) {
            const currentKey = apiKeys[i]['api-key'];
            const duplicateIndex = apiKeys.findIndex((key, index) => index !== i && key['api-key'] === currentKey);
            if (duplicateIndex !== -1) {
                const apiKeyRows = apiKeysContainer.querySelectorAll('.dynamic-input-row');
                if (apiKeyRows[i]) {
                    const apiKeyInput = apiKeyRows[i].querySelector('.dynamic-input:first-child');
                    if (apiKeyInput) showProviderFieldError(apiKeyInput, 'This API key already exists');
                }
                hasErrors = true;
                break;
            }
        }
    }
    if (!hasErrors && models.length > 0) {
        for (let i = models.length - 1; i >= 0; i--) {
            const currentAlias = models[i].alias;
            const duplicateIndex = models.findIndex((model, index) => index !== i && model.alias === currentAlias);
            if (duplicateIndex !== -1) {
                const modelRows = modelsContainer.querySelectorAll('.dynamic-input-row');
                if (modelRows[i]) {
                    const aliasInput = modelRows[i].querySelector('.dynamic-input:nth-child(2)');
                    if (aliasInput) showProviderFieldError(aliasInput, 'This alias name already exists');
                }
                hasErrors = true;
                break;
            }
        }
    }
    if (!hasErrors) {
        const isDuplicate = openaiProviders.some((provider, index) => index !== currentProviderEditIndex && provider.name === name);
        if (isDuplicate) {
            showProviderFieldError(providerNameInput, 'This provider name already exists');
            hasErrors = true;
        }
    }
    if (hasErrors) return;
    const providerData = { name: name, 'base-url': baseUrl, 'api-key-entries': apiKeys, models: models };
    if (currentProviderEditIndex !== null) {
        openaiProviders[currentProviderEditIndex] = providerData;
    } else {
        openaiProviders.push(providerData);
    }
    renderOpenaiProviders();
    hideOpenaiProviderModal();
}

function editOpenaiProvider(index) { showOpenaiProviderModal(index); }
function deleteOpenaiProvider(index) {
    showConfirmDialog(
        'Confirm Delete',
        'Are you sure you want to delete this OpenAI compatibility provider? This action cannot be undone.',
        () => { openaiProviders.splice(index, 1); renderOpenaiProviders(); }
    );
}

function showProviderFieldError(input, message) {
    input.classList.add('error');
    input.focus();
    showError(message);
}

function clearProviderFormErrors() {
    providerNameInput.classList.remove('error');
    providerBaseUrlInput.classList.remove('error');
    const allDynamicInputs = document.querySelectorAll('.dynamic-input');
    allDynamicInputs.forEach(input => input.classList.remove('error'));
}

// Dynamic inputs: API keys and models
function addApiKeyRow() {
    const container = apiKeysContainer;
    const newRow = document.createElement('div');
    newRow.className = 'dynamic-input-row';

    const apiKeyInput = document.createElement('input');
    apiKeyInput.type = 'text';
    apiKeyInput.className = 'form-input dynamic-input';
    apiKeyInput.placeholder = 'Enter API key';
    apiKeyInput.required = true;

    const proxyUrlInput = document.createElement('input');
    proxyUrlInput.type = 'text';
    proxyUrlInput.className = 'form-input dynamic-input';
    proxyUrlInput.placeholder = 'Proxy URL (optional)';

    const addBtn = document.createElement('button');
    addBtn.type = 'button';
    addBtn.className = 'add-row-btn';
    addBtn.textContent = '+';
    addBtn.onclick = addApiKeyRow;

    const removeBtn = document.createElement('button');
    removeBtn.type = 'button';
    removeBtn.className = 'remove-row-btn';
    removeBtn.textContent = '−';
    removeBtn.onclick = () => removeApiKeyRow(newRow);

    newRow.appendChild(apiKeyInput);
    newRow.appendChild(proxyUrlInput);
    newRow.appendChild(addBtn);
    newRow.appendChild(removeBtn);
    container.appendChild(newRow);
    apiKeyInput.focus();
}

function removeApiKeyRow(row) {
    const container = apiKeysContainer;
    if (container.children.length > 1) {
        row.remove();
    } else {
        const inputs = row.querySelectorAll('.dynamic-input');
        inputs.forEach(input => input.value = '');
    }
}

function addModelRow() {
    const container = modelsContainer;
    const newRow = document.createElement('div');
    newRow.className = 'dynamic-input-row';
    const modelNameInput = document.createElement('input');
    modelNameInput.type = 'text';
    modelNameInput.className = 'form-input dynamic-input';
    modelNameInput.placeholder = 'Model name';
    const aliasInput = document.createElement('input');
    aliasInput.type = 'text';
    aliasInput.className = 'form-input dynamic-input';
    aliasInput.placeholder = 'Alias name';
    const addBtn = document.createElement('button');
    addBtn.type = 'button';
    addBtn.className = 'add-row-btn';
    addBtn.textContent = '+';
    const removeBtn = document.createElement('button');
    removeBtn.type = 'button';
    removeBtn.className = 'remove-row-btn';
    removeBtn.textContent = '−';
    removeBtn.onclick = () => removeModelRow(newRow);
    addBtn.onclick = addModelRow;
    newRow.appendChild(modelNameInput);
    newRow.appendChild(aliasInput);
    newRow.appendChild(addBtn);
    newRow.appendChild(removeBtn);
    container.appendChild(newRow);
    modelNameInput.focus();
}

function removeModelRow(row) {
    const container = modelsContainer;
    if (container.children.length > 1) {
        row.remove();
    } else {
        const inputs = row.querySelectorAll('.dynamic-input');
        inputs.forEach(input => input.value = '');
    }
}

function clearDynamicInputs() {
    apiKeysContainer.innerHTML = '';
    modelsContainer.innerHTML = '';
    addApiKeyRow();
    addModelRow();
}

function populateDynamicInputs(apiKeys, models) {
    // Clear existing inputs
    apiKeysContainer.innerHTML = '';
    modelsContainer.innerHTML = '';

    // Populate API keys
    if (apiKeys && apiKeys.length > 0) {
        apiKeys.forEach(apiKeyEntry => {
            const row = document.createElement('div');
            row.className = 'dynamic-input-row';

            const apiKeyInput = document.createElement('input');
            apiKeyInput.type = 'text';
            apiKeyInput.className = 'form-input dynamic-input';
            apiKeyInput.placeholder = 'Enter API key';
            apiKeyInput.required = true;
            apiKeyInput.value = apiKeyEntry['api-key'] || '';

            const proxyUrlInput = document.createElement('input');
            proxyUrlInput.type = 'text';
            proxyUrlInput.className = 'form-input dynamic-input';
            proxyUrlInput.placeholder = 'Proxy URL (optional)';
            proxyUrlInput.value = apiKeyEntry['proxy-url'] || '';

            const addBtn = document.createElement('button');
            addBtn.type = 'button';
            addBtn.className = 'add-row-btn';
            addBtn.textContent = '+';
            addBtn.onclick = addApiKeyRow;

            const removeBtn = document.createElement('button');
            removeBtn.type = 'button';
            removeBtn.className = 'remove-row-btn';
            removeBtn.textContent = '−';
            removeBtn.onclick = () => removeApiKeyRow(row);

            row.appendChild(apiKeyInput);
            row.appendChild(proxyUrlInput);
            row.appendChild(addBtn);
            row.appendChild(removeBtn);
            apiKeysContainer.appendChild(row);
        });
    } else {
        addApiKeyRow();
    }

    // Populate models
    if (models && models.length > 0) {
        models.forEach(model => {
            const row = document.createElement('div');
            row.className = 'dynamic-input-row';

            const modelNameInput = document.createElement('input');
            modelNameInput.type = 'text';
            modelNameInput.className = 'form-input dynamic-input';
            modelNameInput.placeholder = 'Model name';
            modelNameInput.value = model.name || '';

            const aliasInput = document.createElement('input');
            aliasInput.type = 'text';
            aliasInput.className = 'form-input dynamic-input';
            aliasInput.placeholder = 'Alias name';
            aliasInput.value = model.alias || '';

            const addBtn = document.createElement('button');
            addBtn.type = 'button';
            addBtn.className = 'add-row-btn';
            addBtn.textContent = '+';
            addBtn.onclick = addModelRow;

            const removeBtn = document.createElement('button');
            removeBtn.type = 'button';
            removeBtn.className = 'remove-row-btn';
            removeBtn.textContent = '−';
            removeBtn.onclick = () => removeModelRow(row);

            row.appendChild(modelNameInput);
            row.appendChild(aliasInput);
            row.appendChild(addBtn);
            row.appendChild(removeBtn);
            modelsContainer.appendChild(row);
        });
    } else {
        addModelRow();
    }
}

function getDynamicInputData() {
    const apiKeys = Array.from(apiKeysContainer.querySelectorAll('.dynamic-input-row'))
        .map(row => {
            const inputs = row.querySelectorAll('.dynamic-input');
            const apiKey = inputs[0]?.value.trim();
            const proxyUrl = inputs[1]?.value.trim();
            if (!apiKey) return null;
            const entry = { 'api-key': apiKey };
            if (proxyUrl) entry['proxy-url'] = proxyUrl;
            return entry;
        })
        .filter(v => v);
    const models = Array.from(modelsContainer.querySelectorAll('.dynamic-input-row'))
        .map(row => {
            const inputs = row.querySelectorAll('.dynamic-input');
            const name = inputs[0]?.value.trim();
            const alias = inputs[1]?.value.trim();
            if (!name && !alias) return null;
            return { name, alias };
        })
        .filter(v => v);
    return { apiKeys, models };
}

// Events
addProviderBtn.addEventListener('click', () => showOpenaiProviderModal());
providerModalClose.addEventListener('click', hideOpenaiProviderModal);
providerModalCancel.addEventListener('click', hideOpenaiProviderModal);
openaiProviderForm.addEventListener('submit', (e) => { e.preventDefault(); saveOpenaiProvider(); });
openaiProviderModal.addEventListener('click', (e) => { if (e.target === openaiProviderModal) hideOpenaiProviderModal(); });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && openaiProviderModal.classList.contains('show')) hideOpenaiProviderModal(); });

providerNameInput.addEventListener('input', () => { if (providerNameInput.classList.contains('error')) providerNameInput.classList.remove('error'); });
providerBaseUrlInput.addEventListener('input', () => { if (providerBaseUrlInput.classList.contains('error')) providerBaseUrlInput.classList.remove('error'); });
document.addEventListener('input', (e) => { if (e.target.classList.contains('dynamic-input') && e.target.classList.contains('error')) e.target.classList.remove('error'); });

