// Authentication files management: list, selection, upload/download, and actions

// Elements
const selectAllBtn = document.getElementById('select-all-btn');
const deleteBtn = document.getElementById('delete-btn');
const authFilesList = document.getElementById('auth-files-list');
const authLoading = document.getElementById('auth-loading');

// New dropdown elements
const newDropdown = document.getElementById('new-dropdown');
const newBtn = document.getElementById('new-btn');
const dropdownMenu = document.getElementById('dropdown-menu');
const downloadBtn = document.getElementById('download-btn');

// State
let selectedAuthFiles = new Set();
let authFiles = [];

// Load auth files from server
async function loadAuthFiles() {
    try {
        authFiles = await configManager.getAuthFiles();
        renderAuthFiles();
        updateActionButtons();
    } catch (error) {
        console.error('Error loading auth files:', error);
        showError('Network error');
        showEmptyAuthFiles();
        updateActionButtons();
    }
}

// Render auth files list
function renderAuthFiles() {
    authLoading.style.display = 'none';
    if (authFiles.length === 0) {
        showEmptyAuthFiles();
        return;
    }
    authFilesList.innerHTML = '';
    authFiles.forEach(file => {
        const fileItem = document.createElement('div');
        fileItem.className = 'auth-file-item';
        fileItem.dataset.filename = file.name;

        const fileSize = formatFileSize(file.size);
        const modTime = formatDate(file.modtime);

        fileItem.innerHTML = `
            <div class="auth-file-info">
                <div class="auth-file-name">${file.name}</div>
                <div class="auth-file-details">
                    <span class="auth-file-type">Type: ${file.type || 'unknown'}</span>
                    <span class="auth-file-size">${fileSize}</span>
                    <span>Modified: ${modTime}</span>
                </div>
            </div>
        `;

        fileItem.addEventListener('click', () => toggleAuthFileSelection(file.name, fileItem));
        authFilesList.appendChild(fileItem);
    });
}

// Empty state for auth files
function showEmptyAuthFiles() {
    authLoading.style.display = 'none';
    authFilesList.innerHTML = `
        <div class="empty-state">
            <div class="empty-state-icon">📁</div>
            <div class="empty-state-text">No authentication files</div>
            <div class="empty-state-subtitle">Upload authentication files to manage them here</div>
        </div>
    `;
    updateActionButtons();
}

// Toggle selection of an auth file
function toggleAuthFileSelection(filename, fileItem) {
    if (selectedAuthFiles.has(filename)) {
        selectedAuthFiles.delete(filename);
        fileItem.classList.remove('selected');
    } else {
        selectedAuthFiles.add(filename);
        fileItem.classList.add('selected');
    }
    updateActionButtons();
}

// Update action buttons based on current tab/state
function updateActionButtons() {
    const hasSelection = selectedAuthFiles.size > 0;
    const allSelected = selectedAuthFiles.size === authFiles.length && authFiles.length > 0;
    const currentTab = document.querySelector('.tab.active').getAttribute('data-tab');
    if (currentTab === 'auth') {
        resetBtn.style.display = 'none';
        applyBtn.style.display = 'none';
        selectAllBtn.style.display = 'block';
        deleteBtn.style.display = 'block';
        newDropdown.style.display = 'block';
        downloadBtn.style.display = 'block';
        selectAllBtn.textContent = allSelected ? 'Unselect All' : 'Select All';
        deleteBtn.disabled = !hasSelection;
        downloadBtn.disabled = !hasSelection;
    } else if (currentTab === 'access-token' || currentTab === 'api' || currentTab === 'openai' || currentTab === 'basic') {
        resetBtn.style.display = 'block';
        applyBtn.style.display = 'block';
        selectAllBtn.style.display = 'none';
        deleteBtn.style.display = 'none';
        newDropdown.style.display = 'none';
        downloadBtn.style.display = 'none';
    }
}

// Toggle select all auth files
function toggleSelectAllAuthFiles() {
    const allSelected = selectedAuthFiles.size === authFiles.length;
    if (allSelected) {
        selectedAuthFiles.clear();
        document.querySelectorAll('.auth-file-item').forEach(item => item.classList.remove('selected'));
    } else {
        selectedAuthFiles.clear();
        authFiles.forEach(file => selectedAuthFiles.add(file.name));
        document.querySelectorAll('.auth-file-item').forEach(item => item.classList.add('selected'));
    }
    updateActionButtons();
}

// Delete selected auth files
async function deleteSelectedAuthFiles() {
    if (selectedAuthFiles.size === 0 || deleteBtn.disabled) return;
    const fileCount = selectedAuthFiles.size;
    const fileText = fileCount === 1 ? 'file' : 'files';
    showConfirmDialog(
        'Confirm Delete',
        `Are you sure you want to delete ${fileCount} authentication ${fileText}?\nThis action cannot be undone.`,
        async () => {
            deleteBtn.disabled = true;
            deleteBtn.textContent = 'Deleting...';
            try {
                const result = await configManager.deleteAuthFiles(Array.from(selectedAuthFiles));
                if (result.success) {
                    showSuccessMessage(`Deleted ${result.successCount} file(s) successfully`);
                    selectedAuthFiles.clear();
                    await loadAuthFiles();
                } else {
                    if (result.error) {
                        showError(result.error);
                    } else {
                        showError(`Failed to delete ${result.errorCount} file(s)`);
                    }
                }
            } catch (error) {
                console.error('Error deleting auth files:', error);
                showError('Network error');
            } finally {
                deleteBtn.disabled = false;
                deleteBtn.textContent = 'Delete';
                updateActionButtons();
            }
        }
    );
}

// Toggle dropdown menu visibility
function toggleDropdown() {
    dropdownMenu.classList.toggle('show');
}

// Close dropdown menu
function closeDropdown() {
    dropdownMenu.classList.remove('show');
}

// Create a new auth file by type
function createNewAuthFile(type) {
    const typeNames = {
        'gemini': 'Gemini CLI',
        'gemini-web': 'Gemini WEB',
        'claude': 'Claude Code',
        'codex': 'Codex',
        'qwen': 'Qwen Code',
        'iflow': 'iFlow',
        'local': 'Local File'
    };

    if (type === 'local') {
        uploadLocalFile();
    } else if (type === 'codex') {
        startCodexAuthFlow();
    } else if (type === 'claude') {
        startClaudeAuthFlow();
    } else if (type === 'gemini') {
        showGeminiProjectIdDialog();
    } else if (type === 'gemini-web') {
        showGeminiWebDialog();
    } else if (type === 'qwen') {
        startQwenAuthFlow();
    } else if (type === 'iflow') {
        startIFlowAuthFlow();
    } else {
        console.log(`Creating new ${typeNames[type]} auth file`);
        showSuccessMessage(`Creating new ${typeNames[type]} auth file...`);
    }
}

// Show Gemini Web dialog
function showGeminiWebDialog() {
    const modal = document.createElement('div');
    modal.className = 'modal show';
    modal.id = 'gemini-web-modal';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Gemini WEB Authentication</h3>
                <button class="modal-close" id="gemini-web-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Please enter your Gemini Web cookies:</p>
                    <div class="form-group">
                        <label for="gemini-web-secure-1psid-input">Secure-1PSID:</label>
                        <input type="text" id="gemini-web-secure-1psid-input" class="form-input" placeholder="Enter Secure-1PSID">
                    </div>
                    <div class="form-group">
                        <label for="gemini-web-secure-1psidts-input">Secure-1PSIDTS:</label>
                        <input type="text" id="gemini-web-secure-1psidts-input" class="form-input" placeholder="Enter Secure-1PSIDTS">
                    </div>
                    <div class="form-group">
                        <label for="gemini-web-email-input" style="text-align: left;">Email:</label>
                        <input type="email" id="gemini-web-email-input" class="form-input" placeholder="Enter your email address">
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="gemini-web-confirm-btn" class="btn-primary">Confirm</button>
                        <button type="button" id="gemini-web-cancel-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);
    document.getElementById('gemini-web-modal-close').addEventListener('click', cancelGeminiWebDialog);
    document.getElementById('gemini-web-confirm-btn').addEventListener('click', confirmGeminiWebTokens);
    document.getElementById('gemini-web-cancel-btn').addEventListener('click', cancelGeminiWebDialog);
    document.addEventListener('keydown', handleGeminiWebEscapeKey);
    document.getElementById('gemini-web-secure-1psid-input').focus();
}

// Handle Gemini Web dialog escape key
function handleGeminiWebEscapeKey(e) {
    if (e.key === 'Escape') {
        cancelGeminiWebDialog();
    }
}

// Cancel Gemini Web dialog
function cancelGeminiWebDialog() {
    document.removeEventListener('keydown', handleGeminiWebEscapeKey);
    const modal = document.getElementById('gemini-web-modal');
    if (modal) modal.remove();
}

// Confirm Gemini Web tokens
async function confirmGeminiWebTokens() {
    try {
        const emailInput = document.getElementById('gemini-web-email-input');
        const secure1psidInput = document.getElementById('gemini-web-secure-1psid-input');
        const secure1psidtsInput = document.getElementById('gemini-web-secure-1psidts-input');

        const email = emailInput.value.trim();
        const secure1psid = secure1psidInput.value.trim();
        const secure1psidts = secure1psidtsInput.value.trim();

        if (!email || !secure1psid || !secure1psidts) {
            showError('Please enter email, Secure-1PSID and Secure-1PSIDTS');
            return;
        }

        cancelGeminiWebDialog();

        // Call Management API to save Gemini Web tokens
        const result = await configManager.saveGeminiWebTokens(secure1psid, secure1psidts, email);

        if (result.success) {
            showSuccessMessage('Gemini Web tokens saved successfully');
            // Refresh the auth files list
            await loadAuthFiles();
        } else {
            showError('Failed to save Gemini Web tokens: ' + (result.error || 'Unknown error'));
        }
    } catch (error) {
        console.error('Error saving Gemini Web tokens:', error);
        showError('Failed to save Gemini Web tokens: ' + error.message);
    }
}

// Upload local JSON files
function uploadLocalFile() {
    const fileInput = document.createElement('input');
    fileInput.type = 'file';
    fileInput.accept = '.json';
    fileInput.multiple = true;
    fileInput.style.display = 'none';
    document.body.appendChild(fileInput);
    fileInput.click();
    fileInput.addEventListener('change', async (event) => {
        const files = Array.from(event.target.files);
        if (files.length === 0) {
            document.body.removeChild(fileInput);
            return;
        }
        const invalidFiles = files.filter(file => !file.name.toLowerCase().endsWith('.json'));
        if (invalidFiles.length > 0) {
            showError(`Please select only JSON files. Invalid files: ${invalidFiles.map(f => f.name).join(', ')}`);
            document.body.removeChild(fileInput);
            return;
        }
        try {
            await uploadFilesToServer(files);
            await loadAuthFiles();
        } catch (error) {
            console.error('Error uploading files:', error);
            showError('Failed to upload files');
        } finally {
            document.body.removeChild(fileInput);
        }
    });
}

// Upload multiple files via config manager
async function uploadFilesToServer(files) {
    try {
        const result = await configManager.uploadAuthFiles(files);
        if (result.success && result.successCount > 0) {
            showSuccessMessage(`Uploaded ${result.successCount} file(s) successfully`);
        }
        if (result.errorCount > 0) {
            const errorMessage = result.errors && result.errors.length <= 3
                ? `Failed to upload ${result.errorCount} file(s): ${result.errors.join(', ')}`
                : `Failed to upload ${result.errorCount} file(s)`;
            showError(errorMessage);
        }
        if (result.error) {
            showError(result.error);
        }
    } catch (error) {
        console.error('Error uploading files:', error);
        showError('Failed to upload files');
    }
}

// Legacy single-file upload (kept for compatibility)
async function uploadSingleFile(file, apiUrl, password) {
    console.warn('uploadSingleFile is deprecated, use configManager.uploadAuthFiles() instead');
}

// Download selected auth files
async function downloadSelectedAuthFiles() {
    if (selectedAuthFiles.size === 0 || downloadBtn.disabled) return;
    downloadBtn.disabled = true;
    downloadBtn.textContent = 'Downloading...';
    try {
        const result = await configManager.downloadAuthFiles(Array.from(selectedAuthFiles));
        if (result.success && result.successCount > 0) {
            showSuccessMessage(`Downloaded ${result.successCount} file(s) successfully`);
        }
        if (result.errorCount > 0) {
            showError(`Failed to download ${result.errorCount} file(s)`);
        }
        if (result.error) {
            showError(result.error);
        }
    } catch (error) {
        console.error('Error downloading files:', error);
        showError('Failed to download files');
    } finally {
        downloadBtn.disabled = false;
        downloadBtn.textContent = 'Download';
    }
}

// Legacy single-file download (kept for compatibility)
async function downloadFileToDirectory(filename, directoryHandle, baseUrl, password) {
    console.warn('downloadFileToDirectory is deprecated, use configManager.downloadAuthFiles() instead');
}

// Event wiring for auth files UI
selectAllBtn.addEventListener('click', toggleSelectAllAuthFiles);
deleteBtn.addEventListener('click', deleteSelectedAuthFiles);
downloadBtn.addEventListener('click', downloadSelectedAuthFiles);

newBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    toggleDropdown();
});

document.querySelectorAll('.dropdown-item').forEach(item => {
    item.addEventListener('click', (e) => {
        e.stopPropagation();
        const type = item.getAttribute('data-type');
        createNewAuthFile(type);
        closeDropdown();
    });
});

document.addEventListener('click', (e) => {
    if (!newDropdown.contains(e.target)) {
        closeDropdown();
    }
});

