// Card selection functionality
const localCard = document.getElementById('local-card');
const remoteCard = document.getElementById('remote-card');

localCard.addEventListener('click', () => {
    localCard.classList.add('selected');
    remoteCard.classList.remove('selected');
    updateInputForm('local');
});

remoteCard.addEventListener('click', () => {
    remoteCard.classList.add('selected');
    localCard.classList.remove('selected');
    updateInputForm('remote');
});

function updateInputForm(mode) {
    const remoteUrlSection = document.getElementById('remote-url-section');

    if (mode === 'local') {
        // Local card selected - hide both sections
        remoteUrlSection.style.display = 'none';
    } else {
        // Remote card selected - show remote URL section, hide great-for
        remoteUrlSection.style.display = 'block';
    }
}

// Connect button functionality
const continueBtn = document.getElementById('continue-btn');
const remoteUrlInput = document.getElementById('remote-url-input');
const passwordInput = document.getElementById('password-input');
const errorToast = document.getElementById('error-toast');
const successToast = document.getElementById('success-toast');
const progressContainer = document.getElementById('progress-container');
const progressLabel = document.getElementById('progress-label');
const progressFill = document.getElementById('progress-fill');
const progressText = document.getElementById('progress-text');
const updateDialog = document.getElementById('update-dialog');
const updateDialogMessage = document.getElementById('update-dialog-message');
const updateCancelBtn = document.getElementById('update-cancel-btn');
const updateConfirmBtn = document.getElementById('update-confirm-btn');
const passwordDialog = document.getElementById('password-dialog');
const passwordInput1 = document.getElementById('password-input-1');
const passwordInput2 = document.getElementById('password-input-2');
const passwordCancelBtn = document.getElementById('password-cancel-btn');
const passwordSaveBtn = document.getElementById('password-save-btn');

// Initialize the display state
initializeFromLocalStorage();

// Update dialog event listeners
updateCancelBtn.addEventListener('click', () => {
    updateDialog.classList.remove('show');
    // User chose not to update, go directly to settings page
    // Ensure type is set to local in localStorage
    localStorage.setItem('type', "local");
    if (typeof require !== 'undefined') {
        const { ipcRenderer } = require('electron');
        ipcRenderer.send('open-settings');
    }
});

updateConfirmBtn.addEventListener('click', async () => {
    updateDialog.classList.remove('show');
    // User chose to update, start downloading
    try {
        continueBtn.disabled = true;
        continueBtn.textContent = 'Updating...';

        if (typeof require !== 'undefined') {
            const { ipcRenderer } = require('electron');
            const result = await ipcRenderer.invoke('download-cliproxyapi');

            if (result.success) {
                console.log('CLIProxyAPI updated successfully:', result.path);
                console.log('Version:', result.version);

                // Save local connection to localStorage
                localStorage.setItem('type', "local");
                localStorage.setItem('cliproxyapi-path', result.path);
                localStorage.setItem('cliproxyapi-version', result.version);
                localStorage.removeItem('base-url');
                localStorage.removeItem('password');

                // Check if password needs to be set
                const secretKeyResult = await ipcRenderer.invoke('check-secret-key');
                if (secretKeyResult.needsPassword) {
                    console.log('Password needs to be set:', secretKeyResult.reason);
                    passwordDialog.classList.add('show');
                } else {
                    // Password is set, delay redirect to settings page
                    setTimeout(() => {
                        ipcRenderer.send('open-settings');
                    }, 2000);
                }
            } else {
                showError('Failed to update CLIProxyAPI: ' + result.error);
            }
        }
    } catch (error) {
        console.error('Error updating CLIProxyAPI:', error);
        showError('Error updating CLIProxyAPI: ' + error.message);
    } finally {
        continueBtn.disabled = false;
        continueBtn.textContent = 'Connect';
    }
});

// Password dialog event listeners
passwordCancelBtn.addEventListener('click', () => {
    passwordDialog.classList.remove('show');
    // Clear input fields
    passwordInput1.value = '';
    passwordInput2.value = '';
    // User cancelled, return to login page, do not start CLIProxyAPI
    showError('Password must be set to use Local mode');
});

passwordSaveBtn.addEventListener('click', async () => {
    const password1 = passwordInput1.value.trim();
    const password2 = passwordInput2.value.trim();

    // Validate password
    if (!password1) {
        showError('Please enter password');
        return;
    }

    if (!password2) {
        showError('Please confirm password');
        return;
    }

    if (password1 !== password2) {
        showError('Passwords do not match');
        return;
    }

    if (password1.length < 6) {
        showError('Password must be at least 6 characters');
        return;
    }

    try {
        // Disable save button
        passwordSaveBtn.disabled = true;
        passwordSaveBtn.textContent = 'Saving...';

        if (typeof require !== 'undefined') {
            const { ipcRenderer } = require('electron');
            const result = await ipcRenderer.invoke('update-secret-key', password1);

            if (result.success) {
                showSuccess('Password set successfully!');
                passwordDialog.classList.remove('show');
                // Clear input fields
                passwordInput1.value = '';
                passwordInput2.value = '';

                // Ensure type is set to local in localStorage
                localStorage.setItem('type', "local");

                // Delay redirect to settings page
                setTimeout(() => {
                    ipcRenderer.send('open-settings');
                }, 1000);
            } else {
                showError('Failed to set password: ' + result.error);
            }
        }
    } catch (error) {
        console.error('Error setting password:', error);
        showError('Error setting password: ' + error.message);
    } finally {
        // Restore save button
        passwordSaveBtn.disabled = false;
        passwordSaveBtn.textContent = 'Save';
    }
});

// Listen for download progress updates
if (typeof require !== 'undefined') {
    const { ipcRenderer } = require('electron');

    ipcRenderer.on('download-progress', (event, progressData) => {
        updateProgress(progressData);
    });

    ipcRenderer.on('download-status', (event, statusData) => {
        handleDownloadStatus(statusData);
    });

    // Listen for process start errors
    ipcRenderer.on('process-start-error', (event, errorData) => {
        console.error('CLIProxyAPI process start failed:', errorData);
        showError(`Connection error: ${errorData.error}`);
        if (errorData.reason) {
            showError(`Reason: ${errorData.reason}`);
        }
    });

    // Listen for process abnormal exit
    ipcRenderer.on('process-exit-error', (event, errorData) => {
        console.error('CLIProxyAPI process exited abnormally:', errorData);
        showError(`CLIProxyAPI process exited abnormally, exit code: ${errorData.code}`);
    });
}

function initializeFromLocalStorage() {
    const type = localStorage.getItem('type');
    const baseUrl = localStorage.getItem('base-url');
    const password = localStorage.getItem('password');

    if (type === 'remote' && baseUrl) {
        // Select remote card
        remoteCard.classList.add('selected');
        localCard.classList.remove('selected');

        // Fill in the form fields
        remoteUrlInput.value = baseUrl;
        if (password) {
            passwordInput.value = password;
        }

        // Show remote form
        updateInputForm('remote');
    } else {
        // Default to local
        updateInputForm('local');
    }
}

continueBtn.addEventListener('click', async () => {
    const localSelected = localCard.classList.contains('selected');

    if (localSelected) {
        // Handle local connection logic here
        console.log('Local connection selected');

        try {
            // Disable button during check
            continueBtn.disabled = true;
            continueBtn.textContent = 'Checking...';

            // Check version and download if needed
            if (typeof require !== 'undefined') {
                const { ipcRenderer } = require('electron');
                const result = await ipcRenderer.invoke('check-version-and-download');

                if (result.success) {
                    if (result.needsUpdate) {
                        // Update needed, show update dialog
                        updateDialogMessage.textContent =
                            `Current version: ${result.version}\nLatest version: ${result.latestVersion}\n\nDo you want to update to the latest version?`;
                        updateDialog.classList.add('show');

                        // Save current path information
                        localStorage.setItem('type', "local");
                        localStorage.setItem('cliproxyapi-path', result.path);
                        localStorage.setItem('cliproxyapi-version', result.version);
                        localStorage.removeItem('base-url');
                        localStorage.removeItem('password');
                    } else {
                        // Version is latest, check password
                        console.log('CLIProxyAPI version is latest:', result.version);

                        // Save local connection to localStorage
                        localStorage.setItem('type', "local");
                        localStorage.setItem('cliproxyapi-path', result.path);
                        localStorage.setItem('cliproxyapi-version', result.version);
                        localStorage.removeItem('base-url');
                        localStorage.removeItem('password');

                        // Check if password needs to be set
                        const secretKeyResult = await ipcRenderer.invoke('check-secret-key');
                        if (secretKeyResult.needsPassword) {
                            console.log('Password needs to be set:', secretKeyResult.reason);
                            passwordDialog.classList.add('show');
                        } else {
                            // Password is set, go directly to settings page
                            ipcRenderer.send('open-settings');
                        }
                    }
                } else {
                    showError('Failed to check version: ' + result.error);
                }
            } else {
                // Fallback for web environment
                showError('This feature requires running in Electron environment');
            }
        } catch (error) {
            console.error('Error checking version:', error);
            showError('Error checking version: ' + error.message);
        } finally {
            // Re-enable button
            continueBtn.disabled = false;
            continueBtn.textContent = 'Connect';
        }
        return;
    }

    // Handle remote connection
    const remoteUrl = remoteUrlInput.value.trim();
    const password = passwordInput.value.trim();

    if (!remoteUrl) {
        showError('Please enter a remote URL');
        return;
    }

    if (!password) {
        showError('Please enter a password');
        return;
    }

    try {
        // Disable button during request
        continueBtn.disabled = true;
        continueBtn.textContent = 'Connecting...';

        // Save connection info to localStorage first
        localStorage.setItem('type', "remote");
        localStorage.setItem('base-url', remoteUrl);
        localStorage.setItem('password', password);

        // Refresh config manager with new connection info
        configManager.refreshConnection();

        // Test connection by getting config
        try {
            const config = await configManager.getConfig();
            console.log('Connection successful, config loaded');
        } catch (error) {
            if (error.message.includes('401')) {
                showError('Password incorrect');
            } else {
                showError('Server address error');
            }
            return;
        }

        console.log('Connection successful, data saved to localStorage');

        // Close current window and open settings page
        if (typeof require !== 'undefined') {
            const { ipcRenderer } = require('electron');
            ipcRenderer.send('open-settings');
        } else {
            // Fallback for web environment
            window.location.href = 'settings.html';
        }

    } catch (error) {
        console.error('Connection error:', error);
        showError('Server address error');
    } finally {
        // Re-enable button
        continueBtn.disabled = false;
        continueBtn.textContent = 'Connect';
    }
});

// Toast queue management
let toastQueue = [];
let isShowingToast = false;

function showError(message) {
    addToQueue('error', message);
}

function showSuccess(message) {
    addToQueue('success', message);
}

function addToQueue(type, message) {
    toastQueue.push({ type, message });
    if (!isShowingToast) {
        showNextToast();
    }
}

function showNextToast() {
    if (toastQueue.length === 0) {
        isShowingToast = false;
        return;
    }

    isShowingToast = true;
    const { type, message } = toastQueue.shift();
    const toast = type === 'error' ? errorToast : successToast;

    toast.textContent = message;
    toast.classList.add('show');

    // Hide after 3 seconds
    setTimeout(() => {
        toast.classList.remove('show');
        // Wait for animation to complete before showing next toast
        setTimeout(() => {
            showNextToast();
        }, 300); // Wait for CSS animation to complete
    }, 3000);
}

function updateProgress(progressData) {
    const progress = Math.round(progressData.progress);
    const downloaded = formatBytes(progressData.downloaded);
    const total = formatBytes(progressData.total);

    progressFill.style.width = progress + '%';
    progressText.textContent = `${progress}% (${downloaded}/${total})`;
}

function formatBytes(bytes) {
    if (bytes === 0) return '0 Bytes';
    const k = 1024;
    const sizes = ['Bytes', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

function handleDownloadStatus(statusData) {
    switch (statusData.status) {
        case 'checking':
            progressContainer.classList.add('show');
            progressLabel.textContent = 'Checking version...';
            progressFill.style.width = '0%';
            progressText.textContent = '0%';
            break;

        case 'starting':
            progressContainer.classList.add('show');
            progressLabel.textContent = 'Downloading CLIProxyAPI...';
            progressFill.style.width = '0%';
            progressText.textContent = '0%';
            break;

        case 'completed':
            progressLabel.textContent = 'Download completed!';
            progressFill.style.width = '100%';
            progressText.textContent = '100%';
            showSuccess(`CLIProxyAPI ${statusData.version} downloaded and extracted successfully!`);

            // Hide progress bar
            setTimeout(() => {
                progressContainer.classList.remove('show');
            }, 2000);
            break;

        case 'latest':
            progressContainer.classList.remove('show');
            showSuccess(`CLIProxyAPI ${statusData.version} is already the latest version!`);
            break;

        case 'update-available':
            progressContainer.classList.remove('show');
            // Update dialog is handled in main logic
            break;

        case 'failed':
            progressContainer.classList.remove('show');
            showError('Operation failed: ' + statusData.error);
            break;
    }
}
