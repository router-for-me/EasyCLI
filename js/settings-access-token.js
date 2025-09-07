// Access Token management for local/remote modes

// Elements
const addLocalApiKeyBtn = document.getElementById('add-local-api-key-btn');
const addRemoteApiKeyBtn = document.getElementById('add-remote-api-key-btn');
const accessTokenModal = document.getElementById('access-token-modal');
const accessTokenModalTitle = document.getElementById('access-token-modal-title');
const accessTokenForm = document.getElementById('access-token-form');
const accessTokenInput = document.getElementById('access-token-input');
const accessTokenModalClose = document.getElementById('access-token-modal-close');
const accessTokenModalCancel = document.getElementById('access-token-modal-cancel');
const accessTokenModalSave = document.getElementById('access-token-modal-save');

// State
let accessTokenKeys = [];
let originalAccessTokenKeys = [];
let currentAccessTokenEditIndex = null;
let currentAccessTokenMode = null; // 'local' or 'remote'

// Load Access Token keys
async function loadAccessTokenKeys() {
    try {
        accessTokenKeys = await configManager.getApiKeys('access-token');
        originalAccessTokenKeys = JSON.parse(JSON.stringify(accessTokenKeys));
        renderAccessTokenKeys();
    } catch (error) {
        console.error('Error loading Access Token keys:', error);
        showError('Failed to load Access Token keys');
        renderAccessTokenKeys();
    }
}

function renderAccessTokenKeys() {
    const connectionType = localStorage.getItem('type') || 'local';
    const localSection = document.getElementById('local-api-keys-section');
    const remoteSection = document.getElementById('remote-api-keys-section');
    if (connectionType === 'local') {
        localSection.style.display = 'block';
        remoteSection.style.display = 'none';
        renderAccessTokenKeysList('local');
    } else {
        localSection.style.display = 'none';
        remoteSection.style.display = 'block';
        renderAccessTokenKeysList('remote');
    }
}

function renderAccessTokenKeysList(mode) {
    const listId = mode === 'local' ? 'local-api-keys-list' : 'remote-api-keys-list';
    const loadingId = mode === 'local' ? 'local-api-keys-loading' : 'remote-api-keys-loading';
    const loading = document.getElementById(loadingId);
    const list = document.getElementById(listId);
    if (!list) return;
    if (loading) loading.style.display = 'none';

    if (accessTokenKeys.length === 0) {
        list.innerHTML = `
            <div class="empty-state">
                <div class="empty-state-icon">🔑</div>
                <div class="empty-state-text">No Access Tokens</div>
                <div class="empty-state-subtitle">Add your first access token to get started</div>
            </div>
        `;
        return;
    }

    list.innerHTML = '';
    accessTokenKeys.forEach((key, index) => {
        const keyItem = document.createElement('div');
        keyItem.className = 'api-key-item';
        keyItem.innerHTML = `
            <div class="api-key-info">
                <div class="api-key-value">${key}</div>
            </div>
            <div class="api-key-actions">
                <button class="api-key-btn edit" onclick="editAccessTokenKey(${index})">Edit</button>
                <button class="api-key-btn delete" onclick="deleteAccessTokenKey(${index})">Delete</button>
            </div>
        `;
        list.appendChild(keyItem);
    });
}

function showAccessTokenModal(mode, editIndex = null) {
    currentAccessTokenMode = mode;
    currentAccessTokenEditIndex = editIndex;
    accessTokenModalTitle.textContent = editIndex !== null ? 'Edit Access Token' : 'Add Access Token';
    accessTokenInput.value = '';
    clearAccessTokenFormErrors();
    if (editIndex !== null) {
        accessTokenInput.value = accessTokenKeys[editIndex];
    }
    accessTokenModal.classList.add('show');
    accessTokenInput.focus();
}

function hideAccessTokenModal() {
    accessTokenModal.classList.remove('show');
    currentAccessTokenMode = null;
    currentAccessTokenEditIndex = null;
}

function saveAccessTokenKey() {
    const apiKey = accessTokenInput.value.trim();
    const currentTab = document.querySelector('.tab.active').getAttribute('data-tab');
    if (currentTab !== 'access-token') {
        showError('Please switch to Access Token tab to manage access tokens');
        return;
    }
    clearAccessTokenFormErrors();
    let hasErrors = false;
    if (!apiKey) {
        showAccessTokenFieldError(accessTokenInput, 'Please fill in this field');
        hasErrors = true;
    }
    if (!hasErrors) {
        const isDuplicate = accessTokenKeys.some((key, index) => index !== currentAccessTokenEditIndex && key === apiKey);
        if (isDuplicate) {
            showAccessTokenFieldError(accessTokenInput, 'This access token already exists');
            hasErrors = true;
        }
    }
    if (hasErrors) return;
    if (currentAccessTokenEditIndex !== null) {
        accessTokenKeys[currentAccessTokenEditIndex] = apiKey;
    } else {
        accessTokenKeys.push(apiKey);
    }
    renderAccessTokenKeys();
    hideAccessTokenModal();
}

function showAccessTokenFieldError(input, message) {
    input.classList.add('error');
    input.focus();
    showError(message);
}

function clearAccessTokenFormErrors() {
    accessTokenInput.classList.remove('error');
}

function editAccessTokenKey(index) {
    const connectionType = localStorage.getItem('type') || 'local';
    showAccessTokenModal(connectionType, index);
}

function deleteAccessTokenKey(index) {
    showConfirmDialog(
        'Confirm Delete',
        'Are you sure you want to delete this access token?\nThis action cannot be undone.',
        () => {
            accessTokenKeys.splice(index, 1);
            renderAccessTokenKeys();
        }
    );
}

// Wire modal events
accessTokenModalClose.addEventListener('click', hideAccessTokenModal);
accessTokenModalCancel.addEventListener('click', hideAccessTokenModal);
accessTokenForm.addEventListener('submit', (e) => { e.preventDefault(); saveAccessTokenKey(); });
accessTokenModalSave.addEventListener('click', (e) => { e.preventDefault(); saveAccessTokenKey(); });
accessTokenModal.addEventListener('click', (e) => { if (e.target === accessTokenModal) hideAccessTokenModal(); });

// Clear errors when user types
accessTokenInput.addEventListener('input', () => { if (accessTokenInput.classList.contains('error')) accessTokenInput.classList.remove('error'); });

// Buttons
addLocalApiKeyBtn.addEventListener('click', () => showAccessTokenModal('local'));
addRemoteApiKeyBtn.addEventListener('click', () => showAccessTokenModal('remote'));

