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
    const proxySection = document.getElementById('proxy-section');

    if (mode === 'local') {
        // Local card selected - hide remote URL section, show proxy section
        remoteUrlSection.style.display = 'none';
        proxySection.style.display = 'block';
    } else {
        // Remote card selected - show remote URL section, hide proxy section
        remoteUrlSection.style.display = 'block';
        proxySection.style.display = 'none';
    }
}

// Connect button functionality
const continueBtn = document.getElementById('continue-btn');
const remoteUrlInput = document.getElementById('remote-url-input');
const passwordInput = document.getElementById('password-input');
const proxyInput = document.getElementById('proxy-input');
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

// Proxy server validation function
function validateProxyUrl(proxyUrl) {
    if (!proxyUrl || proxyUrl.trim() === '') {
        return { valid: true, error: null }; // Empty proxy is valid (optional)
    }

    const trimmedUrl = proxyUrl.trim();

    // Enhanced regex patterns to match backend parsing logic
    // Support both formats: protocol://host:port and protocol://user:pass@host:port
    const httpProxyRegex = /^https?:\/\/[^:\s@]+:\d+$/;
    const httpProxyWithAuthRegex = /^https?:\/\/[^:\s]+:[^:\s]+@[^:\s]+:\d+$/;
    const socks5ProxyRegex = /^socks5:\/\/[^:\s@]+:\d+$/;
    const socks5WithAuthRegex = /^socks5:\/\/[^:\s]+:[^:\s]+@[^:\s]+:\d+$/;

    if (httpProxyRegex.test(trimmedUrl) ||
        httpProxyWithAuthRegex.test(trimmedUrl) ||
        socks5ProxyRegex.test(trimmedUrl) ||
        socks5WithAuthRegex.test(trimmedUrl)) {
        return { valid: true, error: null };
    }

    return {
        valid: false,
        error: 'Invalid proxy format. Supported formats: http://host:port, https://host:port, socks5://host:port, http://user:pass@host:port, https://user:pass@host:port, socks5://user:pass@host:port'
    };
}

// Update dialog event listeners
async function openSettingsWindowPreferNew() {
    try {
        if (window.__TAURI__?.core?.invoke) {
            await window.__TAURI__.core.invoke('open_settings_window');
            // Backend command closes the login window; avoid double-close that could exit the app.
            return;
        }
    } catch (e) {
        console.error('open_settings_window failed:', e);
    }
    // Fallback only if Tauri unavailable
    window.location.href = 'settings.html';
}
updateCancelBtn.addEventListener('click', async () => {
    updateDialog.classList.remove('show');
    // User chose not to update, still run local
    localStorage.setItem('type', "local");
    if (window.__TAURI__?.core?.invoke) {
        try {
            const startRes = await window.__TAURI__.core.invoke('start_cliproxyapi');
            if (!startRes || !startRes.success) {
                showError('CLIProxyAPI process start failed');
                return;
            }
            // Save the generated password for local mode HTTP requests
            if (startRes.password) {
                localStorage.setItem('local-management-key', startRes.password);
                console.log('Saved local management key:', startRes.password);
            }
            // Start keep-alive mechanism for Local mode
            if (window.configManager) {
                window.configManager.startKeepAlive().catch(error => {
                    console.error('Error starting keep-alive:', error);
                });
            }
        } catch (e) {
            showError('CLIProxyAPI process start error');
            return;
        }
        await openSettingsWindowPreferNew();
    }
});

updateConfirmBtn.addEventListener('click', async () => {
    updateDialog.classList.remove('show');
    // User chose to update, start downloading
    try {
        continueBtn.disabled = true;
        continueBtn.textContent = 'Updating...';

        if (window.__TAURI__?.core?.invoke) {
            const proxyUrl = proxyInput.value.trim();
            const result = await window.__TAURI__.core.invoke('download_cliproxyapi', proxyUrl);

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
                const secretKeyResult = await window.__TAURI__.core.invoke('check_secret_key');
                if (secretKeyResult.needsPassword) {
                    console.log('Password needs to be set:', secretKeyResult.reason);
                    passwordDialog.classList.add('show');
                } else {
                    // Password is set: start process then go to settings
                    try {
                        const startRes = await window.__TAURI__.core.invoke('start_cliproxyapi');
                        if (!startRes || !startRes.success) {
                            showError('CLIProxyAPI process start failed');
                            return;
                        }
                        // Save the generated password for local mode HTTP requests
                        if (startRes.password) {
                            localStorage.setItem('local-management-key', startRes.password);
                            console.log('Saved local management key:', startRes.password);
                        }
                        // Start keep-alive mechanism for Local mode
                        if (window.configManager) {
                            window.configManager.startKeepAlive().catch(error => {
                                console.error('Error starting keep-alive:', error);
                            });
                        }
                    } catch (e) {
                        showError('CLIProxyAPI process start error');
                        return;
                    }
                    setTimeout(async () => { await openSettingsWindowPreferNew(); }, 800);
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

        if (window.__TAURI__?.core?.invoke) {
            const result = await window.__TAURI__.core.invoke('update_secret_key', {
                args: { secret_key: password1 },
            });

            if (result.success) {
                showSuccess('Password set successfully!');
                passwordDialog.classList.remove('show');
                // Clear input fields
                passwordInput1.value = '';
                passwordInput2.value = '';

                // Ensure type is set to local in localStorage
                localStorage.setItem('type', "local");

                // Start process then go to settings
                try {
                    const startRes = await window.__TAURI__.core.invoke('start_cliproxyapi');
                    if (!startRes || !startRes.success) {
                        showError('CLIProxyAPI process start failed');
                        return;
                    }
                    // Save the generated password for local mode HTTP requests
                    if (startRes.password) {
                        localStorage.setItem('local-management-key', startRes.password);
                        console.log('Saved local management key:', startRes.password);
                    }
                } catch (e) {
                    showError('CLIProxyAPI process start error');
                    return;
                }
                setTimeout(async () => { await openSettingsWindowPreferNew(); }, 600);
            } else {
                showError('Failed to set password: ' + result.error);
            }
        }
    } catch (error) {
        console.error('Error setting password:', error);
        // Handle different error types
        let errorMessage = 'Unknown error';
        if (error && typeof error === 'string') {
            errorMessage = error;
        } else if (error && error.message) {
            errorMessage = error.message;
        } else if (error && error.toString) {
            errorMessage = error.toString();
        }
        showError('Error setting password: ' + errorMessage);
    } finally {
        // Restore save button
        passwordSaveBtn.disabled = false;
        passwordSaveBtn.textContent = 'Save';
    }
});

// Listen for download progress updates
if (window.__TAURI__?.event?.listen) {
    window.__TAURI__.event.listen('download-progress', (event) => { updateProgress(event?.payload || {}); });
    window.__TAURI__.event.listen('download-status', (event) => { handleDownloadStatus(event?.payload || {}); });
    window.__TAURI__.event.listen('process-start-error', (event) => {
        const errorData = event?.payload || {};
        console.error('CLIProxyAPI process start failed:', errorData);
        showError(`Connection error: ${errorData.error}`);
        if (errorData.reason) showError(`Reason: ${errorData.reason}`);
    });
    window.__TAURI__.event.listen('process-exit-error', (event) => {
        const errorData = event?.payload || {};
        console.error('CLIProxyAPI process exited abnormally:', errorData);
        showError(`CLIProxyAPI process exited abnormally, exit code: ${errorData.code}`);
    });
}

function initializeFromLocalStorage() {
    const type = localStorage.getItem('type');
    const baseUrl = localStorage.getItem('base-url');
    const password = localStorage.getItem('password');
    const proxyUrl = localStorage.getItem('proxy-url');

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

        // Fill in proxy field if exists
        if (proxyUrl) {
            proxyInput.value = proxyUrl;
        }
    }
}

async function handleConnectClick() {
    try { showSuccess('Connecting...'); } catch (_) { }
    const localSelected = localCard.classList.contains('selected');

    if (localSelected) {
        // Handle local connection logic here
        console.log('Local connection selected');

        // Validate proxy server URL if provided
        const proxyUrl = proxyInput.value.trim();
        if (proxyUrl) {
            const validation = validateProxyUrl(proxyUrl);
            if (!validation.valid) {
                showError(validation.error);
                return;
            }
        }

        try {
            // Disable button during check
            continueBtn.disabled = true;
            continueBtn.textContent = 'Checking...';

            // Save proxy URL to localStorage
            if (proxyUrl) {
                localStorage.setItem('proxy-url', proxyUrl);
            } else {
                localStorage.removeItem('proxy-url');
            }

            // Check version and download if needed
            if (window.__TAURI__?.core?.invoke) {
                const result = await window.__TAURI__.core.invoke('check_version_and_download', proxyUrl);

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
                        const secretKeyResult = await window.__TAURI__.core.invoke('check_secret_key');
                        if (secretKeyResult.needsPassword) {
                            console.log('Password needs to be set:', secretKeyResult.reason);
                            passwordDialog.classList.add('show');
                        } else {
                            // Password is set: start process then open settings page
                            try {
                                const startRes = await window.__TAURI__.core.invoke('start_cliproxyapi');
                                if (!startRes || !startRes.success) {
                                    showError('CLIProxyAPI process start failed');
                                    return;
                                }
                                // Save the generated password for local mode HTTP requests
                                if (startRes.password) {
                                    localStorage.setItem('local-management-key', startRes.password);
                                    console.log('Saved local management key:', startRes.password);
                                }
                            } catch (e) {
                                showError('CLIProxyAPI process start error');
                                return;
                            }
                            await openSettingsWindowPreferNew();
                        }
                    }
                } else {
                    showError('Failed to check version: ' + result.error);
                }
            } else {
                // Fallback for non-Tauri environment
                showError('This feature requires Tauri environment');
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

        console.log('=== DEBUG: Connection attempt ===');
        console.log('Input remoteUrl:', remoteUrl);
        console.log('Saved to localStorage base-url:', localStorage.getItem('base-url'));
        console.log('Saved to localStorage type:', localStorage.getItem('type'));

        // Clear any cached config to ensure fresh connection
        localStorage.removeItem('config');

        // Create a fresh config manager instance to ensure no caching issues
        const freshConfigManager = new ConfigManager();
        console.log('Fresh configManager baseUrl:', freshConfigManager.baseUrl);
        console.log('Fresh configManager type:', freshConfigManager.type);

        // Test connection by getting config with fresh instance
        try {
            const config = await freshConfigManager.getConfig();
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
        await openSettingsWindowPreferNew();

    } catch (error) {
        console.error('Connection error:', error);
        showError('Server address error');
    } finally {
        // Re-enable button
        continueBtn.disabled = false;
        continueBtn.textContent = 'Connect';
    }
}

// Attach click handler safely and expose a fallback hook
if (continueBtn) {
    continueBtn.addEventListener('click', handleConnectClick);
}
// Provide a global fallback for inline onclick
window.__onConnect = handleConnectClick;

// Event delegation fallback in case of dynamic DOM
document.addEventListener('click', (e) => {
    const t = e.target;
    if (t && t.id === 'continue-btn') {
        handleConnectClick();
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
