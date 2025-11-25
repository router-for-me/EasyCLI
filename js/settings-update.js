// CLI Update functionality for settings page
// Handles version checking, update dialog, and download progress

// UI Elements
const cliVersionText = document.getElementById('cli-version-text');
const updateCliBtn = document.getElementById('update-cli-btn');
const cliUpdateDialog = document.getElementById('cli-update-dialog');
const cliUpdateClose = document.getElementById('cli-update-close');
const cliUpdateMessage = document.getElementById('cli-update-message');
const cliUpdateDetails = document.getElementById('cli-update-details');
const cliUpdateCancel = document.getElementById('cli-update-cancel');
const cliUpdateConfirm = document.getElementById('cli-update-confirm');
const cliProgressDialog = document.getElementById('cli-progress-dialog');
const cliProgressLabel = document.getElementById('cli-progress-label');
const cliProgressFill = document.getElementById('cli-progress-fill');
const cliProgressText = document.getElementById('cli-progress-text');

// State
let currentVersion = null;
let latestVersion = null;
let updateAvailable = false;

// Format bytes to human readable string
function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
}

// Load and display current CLI version
async function loadCliVersion() {
    const connectionType = localStorage.getItem('type') || 'local';

    // Only show version in Local mode
    if (connectionType !== 'local') {
        return;
    }

    try {
        const storedVersion = localStorage.getItem('cliproxyapi-version');
        if (storedVersion) {
            currentVersion = storedVersion;
            cliVersionText.textContent = `v${currentVersion}`;
        } else {
            cliVersionText.textContent = 'Not installed';
        }
    } catch (error) {
        console.error('Error loading CLI version:', error);
        cliVersionText.textContent = 'Unknown';
    }
}

// Show update dialog
function showUpdateDialog(current, latest) {
    cliUpdateMessage.textContent = `A new version of CLIProxyAPI is available!`;
    cliUpdateDetails.innerHTML = `
        <div style="margin-top: 12px; padding: 12px; background: rgba(16, 185, 129, 0.1); border-radius: 6px;">
            <div style="margin-bottom: 8px;">
                <strong>Current Version:</strong> v${current}
            </div>
            <div>
                <strong>Latest Version:</strong> v${latest}
            </div>
        </div>
    `;
    cliUpdateDialog.classList.add('show');
}

// Hide update dialog
function hideUpdateDialog() {
    cliUpdateDialog.classList.remove('show');
}

// Show progress dialog
function showProgressDialog() {
    cliProgressDialog.classList.add('show');
}

// Hide progress dialog
function hideProgressDialog() {
    cliProgressDialog.classList.remove('show');
}

// Update progress display
function updateProgress(progressData) {
    const percentage = Math.round(progressData.progress || 0);
    cliProgressFill.style.width = `${percentage}%`;

    if (progressData.downloaded && progressData.total) {
        const downloadedStr = formatBytes(progressData.downloaded);
        const totalStr = formatBytes(progressData.total);
        cliProgressText.textContent = `${percentage}% (${downloadedStr} / ${totalStr})`;
    } else {
        cliProgressText.textContent = `${percentage}%`;
    }
}

// Handle download status updates
function handleDownloadStatus(statusData) {
    const status = statusData.status;

    switch (status) {
        case 'checking':
            cliProgressLabel.textContent = 'Checking for updates...';
            updateProgress({ progress: 0 });
            break;

        case 'starting':
            cliProgressLabel.textContent = 'Starting download...';
            updateProgress({ progress: 0 });
            break;

        case 'downloading':
            cliProgressLabel.textContent = 'Downloading CLIProxyAPI...';
            break;

        case 'extracting':
            cliProgressLabel.textContent = 'Extracting files...';
            updateProgress({ progress: 95 });
            break;

        case 'completed':
            cliProgressLabel.textContent = 'Update completed successfully!';
            updateProgress({ progress: 100 });

            // Update version display
            if (statusData.version) {
                currentVersion = statusData.version;
                cliVersionText.textContent = `v${currentVersion}`;
                localStorage.setItem('cliproxyapi-version', currentVersion);
            }

            // Hide progress dialog after a short delay
            setTimeout(() => {
                hideProgressDialog();
                showSuccessMessage('CLIProxyAPI updated successfully! Restarting process...');

                // Restart CLIProxyAPI process
                if (window.__TAURI__?.core?.invoke) {
                    window.__TAURI__.core.invoke('restart_cliproxyapi').catch(error => {
                        console.error('Error restarting CLIProxyAPI:', error);
                        showError('Failed to restart CLIProxyAPI. Please restart manually.');
                    });
                }
            }, 1500);
            break;

        case 'latest':
            hideProgressDialog();
            showSuccessMessage('CLIProxyAPI is already up to date!');
            break;

        case 'update-available':
            hideProgressDialog();
            break;

        case 'failed':
            hideProgressDialog();
            showError(statusData.message || 'Failed to update CLIProxyAPI');
            break;

        default:
            console.log('Unknown status:', status);
    }
}

// Check for updates
async function checkForUpdates() {
    const connectionType = localStorage.getItem('type') || 'local';

    // Only works in Local mode
    if (connectionType !== 'local') {
        showError('Update feature is only available in Local mode');
        return;
    }

    updateCliBtn.disabled = true;
    updateCliBtn.textContent = 'Checking...';

    try {
        showProgressDialog();

        // Get proxy URL from localStorage if available
        const proxyUrl = localStorage.getItem('proxy-url') || null;

        // Call Tauri command to check version
        const result = await window.__TAURI__.core.invoke('check_version_and_download', {
            proxy: proxyUrl
        });

        if (result.success) {
            currentVersion = result.version;
            latestVersion = result.latestVersion;

            if (result.needsUpdate) {
                // Update available
                updateAvailable = true;
                hideProgressDialog();
                showUpdateDialog(currentVersion, latestVersion);
            } else {
                // Already up to date
                updateAvailable = false;
                handleDownloadStatus({ status: 'latest' });
            }
        } else {
            throw new Error('Failed to check for updates');
        }
    } catch (error) {
        console.error('Error checking for updates:', error);
        hideProgressDialog();
        showError('Failed to check for updates: ' + error.message);
    } finally {
        updateCliBtn.disabled = false;
        updateCliBtn.textContent = 'Check Update';
    }
}

// Download and install update
async function downloadUpdate() {
    hideUpdateDialog();
    showProgressDialog();

    try {
        // Get proxy URL from localStorage if available
        const proxyUrl = localStorage.getItem('proxy-url') || null;

        // Call Tauri command to download
        const result = await window.__TAURI__.core.invoke('download_cliproxyapi', {
            proxy: proxyUrl
        });

        if (!result.success) {
            throw new Error(result.message || 'Download failed');
        }
    } catch (error) {
        console.error('Error downloading update:', error);
        hideProgressDialog();
        showError('Failed to download update: ' + error.message);
    }
}

// Event listeners for update button
updateCliBtn.addEventListener('click', checkForUpdates);

// Event listeners for update dialog
cliUpdateClose.addEventListener('click', hideUpdateDialog);
cliUpdateCancel.addEventListener('click', hideUpdateDialog);
cliUpdateConfirm.addEventListener('click', downloadUpdate);

// Listen for download progress events from Tauri
if (window.__TAURI__?.event?.listen) {
    window.__TAURI__.event.listen('download-progress', (event) => {
        updateProgress(event?.payload || {});
    });

    window.__TAURI__.event.listen('download-status', (event) => {
        handleDownloadStatus(event?.payload || {});
    });
}

// Initialize version display on page load
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', loadCliVersion);
} else {
    loadCliVersion();
}

// Re-load version when connection type changes (via storage event)
window.addEventListener('storage', (event) => {
    if (event.key === 'type' || event.key === 'cliproxyapi-version') {
        loadCliVersion();
    }
});
