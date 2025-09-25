// Qwen Code authentication flow

let qwenAuthUrl = null;
let qwenAuthState = null;
let qwenPollingInterval = null;
let qwenAbortController = null;
let qwenCancelled = false;
let qwenPollingActive = false;

async function startQwenAuthFlow() {
    try {
        console.log('Starting Qwen Code authentication flow...');
        await getQwenAuthUrl();
        showQwenAuthDialog();
    } catch (error) {
        console.error('Error starting Qwen auth flow:', error);
        showError('Failed to start Qwen Code authentication flow: ' + error.message);
    }
}

async function getQwenAuthUrl() {
    try {
        const currentMode = localStorage.getItem('type') || 'local';
        let baseUrl, password;

        if (currentMode === 'local') {
            // Read configuration from config.yaml in Local mode
            const config = await configManager.getConfig();
            const port = config.port || 8317; // Default port
            baseUrl = `http://127.0.0.1:${port}`;
            password = localStorage.getItem('local-management-key') || '';
        } else {
            // Read configuration from configManager in Remote mode
            configManager.refreshConnection();
            baseUrl = configManager.baseUrl;
            password = configManager.password;
            if (!baseUrl || !password) throw new Error('Missing connection information');
        }

        const apiUrl = baseUrl.endsWith('/') ? `${baseUrl}v0/management/qwen-auth-url` : `${baseUrl}/v0/management/qwen-auth-url`;
        console.log('Requesting Qwen auth URL from:', apiUrl);
        const headers = currentMode === 'local'
            ? { 'X-Management-Key': password, 'Content-Type': 'application/json' }
            : { 'Authorization': `Bearer ${password}`, 'Content-Type': 'application/json' };
        const response = await fetch(apiUrl, { method: 'GET', headers: headers });
        if (!response.ok) throw new Error(`Failed to get Qwen authentication URL: ${response.status}`);
        const data = await response.json();
        qwenAuthUrl = data.url;
        qwenAuthState = data.state;
        if (!qwenAuthUrl) throw new Error('No valid authentication URL received');
        if (!qwenAuthState) throw new Error('No valid authentication state received');
        console.log('Got Qwen auth URL:', qwenAuthUrl);
        console.log('Got Qwen auth state:', qwenAuthState);
    } catch (error) { console.error('Error getting Qwen auth URL:', error); throw error; }
}

function showQwenAuthDialog() {
    const modal = document.createElement('div');
    modal.id = 'qwen-auth-modal';
    modal.className = 'modal show';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Qwen Code Authentication</h3>
                <button class="modal-close" id="qwen-auth-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Please copy the link below and open it in your browser to complete Qwen Code Authentication:</p>
                    <div class="auth-url-container">
                        <input type="text" id="qwen-auth-url-input" class="form-input" value="${qwenAuthUrl}" readonly>
                        <button type="button" id="qwen-copy-btn" class="copy-btn">Copy Link</button>
                    </div>
                    <div class="auth-status" id="qwen-auth-status" style="display: none;">
                        <div class="auth-status-text">Waiting for authentication to complete...</div>
                        <div class="auth-status-spinner"></div>
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="qwen-open-btn" class="btn-primary">Open Link</button>
                        <button type="button" id="qwen-cancel-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);
    document.getElementById('qwen-auth-modal-close').addEventListener('click', cancelQwenAuth);
    document.getElementById('qwen-copy-btn').addEventListener('click', copyQwenUrl);
    document.getElementById('qwen-open-btn').addEventListener('click', openQwenUrl);
    document.getElementById('qwen-cancel-btn').addEventListener('click', cancelQwenAuth);
    modal.addEventListener('click', (e) => { if (e.target === modal) cancelQwenAuth(); });
    document.addEventListener('keydown', handleQwenEscapeKey);
    const input = document.getElementById('qwen-auth-url-input');
    input.focus(); input.select();

    // Start polling authentication status
    startQwenAuthPolling();
}

async function copyQwenUrl() {
    try { await navigator.clipboard.writeText(qwenAuthUrl); showSuccessMessage('Link copied to clipboard'); }
    catch (error) { console.error('Error copying Qwen URL:', error); showError('Failed to copy link: ' + error.message); }
}

function openQwenUrl() {
    try {
        if (window.__TAURI__?.shell?.open) { window.__TAURI__.shell.open(qwenAuthUrl); }
        else { window.open(qwenAuthUrl, '_blank'); }
        showSuccessMessage('Authentication link opened in browser');

        // Show polling status
        const statusDiv = document.getElementById('qwen-auth-status');
        if (statusDiv) {
            statusDiv.style.display = 'block';
        }
    } catch (error) { console.error('Error opening Qwen URL:', error); showError('Failed to open link: ' + error.message); }
}

// Start Qwen authentication status polling
async function startQwenAuthPolling() {
    if (!qwenAuthState) {
        console.error('No auth state available for polling');
        return;
    }

    try {
        console.log('Starting Qwen authentication polling, state:', qwenAuthState);
        qwenCancelled = false; // Reset cancel flag
        await pollQwenAuthStatus(
            'Qwen Code',
            qwenAuthState,
            () => {
                // Authentication successful
                console.log('Qwen Code Authentication successful');
                showSuccessMessage('Qwen Code Authentication completed!');
                cancelQwenAuth();
                // Refresh auth files list
                if (typeof loadAuthFiles === 'function') {
                    loadAuthFiles();
                }
            },
            (error) => {
                // Authentication failed
                console.error('Qwen Code Authentication failed:', error);
                showError('Qwen Code Authentication failed: ' + error);
                cancelQwenAuth();
            }
        );
    } catch (error) {
        console.error('Qwen Code Authentication polling error:', error);
        showError('Error occurred during Qwen Code Authentication: ' + error.message);
        cancelQwenAuth();
    }
}

function cancelQwenAuth() {
    try {
        console.log('Canceling Qwen authentication, current polling interval ID:', qwenPollingInterval);

        // Set cancel flag
        qwenCancelled = true;
        qwenPollingActive = false;

        document.removeEventListener('keydown', handleQwenEscapeKey);
        const modal = document.getElementById('qwen-auth-modal');
        if (modal) modal.remove();

        // Cancel ongoing requests
        if (qwenAbortController) {
            console.log('Canceling Qwen polling request');
            qwenAbortController.abort();
            qwenAbortController = null;
        }

        if (qwenPollingInterval) {
            console.log('Stopping Qwen polling, interval ID:', qwenPollingInterval);
            clearInterval(qwenPollingInterval);
            qwenPollingInterval = null;
            console.log('Qwen polling stopped');
        } else {
            console.log('No active Qwen polling to stop');
        }
        qwenAuthUrl = null;
        qwenAuthState = null;
    } catch (error) { console.error('Error canceling Qwen auth:', error); }
}

function handleQwenEscapeKey(e) { if (e.key === 'Escape') cancelQwenAuth(); }

// Qwen authentication status polling function
async function pollQwenAuthStatus(authType, state, onSuccess, onError) {
    return new Promise((resolve, reject) => {
        // Create AbortController for canceling requests
        qwenAbortController = new AbortController();

        const pollInterval = setInterval(async () => {
            try {
                console.log('Qwen polling is running...', new Date().toLocaleTimeString(), 'Active status:', qwenPollingActive);

                // Check if already canceled
                if (!qwenPollingActive || qwenCancelled || qwenAbortController.signal.aborted) {
                    console.log('Qwen polling has been canceled, stopping polling');
                    clearInterval(pollInterval);
                    qwenPollingInterval = null;
                    qwenPollingActive = false;
                    return;
                }

                const currentMode = localStorage.getItem('type') || 'local';
                let baseUrl, password;

                if (currentMode === 'local') {
                    const config = await configManager.getConfig();
                    const port = config.port || 8317;
                    baseUrl = `http://127.0.0.1:${port}`;
                    password = localStorage.getItem('local-management-key') || '';
                } else {
                    configManager.refreshConnection();
                    baseUrl = configManager.baseUrl;
                    password = configManager.password;
                    if (!baseUrl || !password) throw new Error('Missing connection information');
                }

                const apiUrl = baseUrl.endsWith('/')
                    ? `${baseUrl}v0/management/get-auth-status?state=${encodeURIComponent(state)}`
                    : `${baseUrl}/v0/management/get-auth-status?state=${encodeURIComponent(state)}`;

                const headers = currentMode === 'local'
                    ? { 'X-Management-Key': password, 'Content-Type': 'application/json' }
                    : { 'Authorization': `Bearer ${password}`, 'Content-Type': 'application/json' };
                const response = await fetch(apiUrl, {
                    method: 'GET',
                    headers: headers,
                    signal: qwenAbortController.signal
                });

                if (!response.ok) {
                    throw new Error(`Failed to get authentication status: ${response.status}`);
                }

                const data = await response.json();
                console.log(`${authType} authentication status:`, data);

                if (data.status === 'ok') {
                    clearInterval(pollInterval);
                    qwenPollingInterval = null;
                    qwenAbortController = null;
                    onSuccess();
                    resolve(data);
                } else if (data.status === 'error') {
                    clearInterval(pollInterval);
                    qwenPollingInterval = null;
                    qwenAbortController = null;
                    onError(data.error || 'Error occurred during authentication');
                    reject(new Error(data.error || 'Error occurred during authentication'));
                }
                // If status is 'wait', continue polling
            } catch (error) {
                if (error.name === 'AbortError') {
                    console.log('Qwen polling request canceled');
                    clearInterval(pollInterval);
                    qwenPollingInterval = null;
                    qwenAbortController = null;
                    return;
                }
                console.error(`Error polling ${authType} authentication status:`, error);
                clearInterval(pollInterval);
                qwenPollingInterval = null;
                qwenAbortController = null;
                onError(error.message);
                reject(error);
            }
        }, 2000); // Poll every 2 seconds

        // Store polling interval ID in global variable for cancellation
        qwenPollingInterval = pollInterval;
        qwenPollingActive = true;
        console.log('Qwen polling started, interval ID:', pollInterval, 'Active status:', qwenPollingActive);

        // Set timeout (5 minutes)
        setTimeout(() => {
            clearInterval(pollInterval);
            qwenPollingInterval = null;
            if (qwenAbortController) {
                qwenAbortController.abort();
                qwenAbortController = null;
            }
            onError('Authentication timeout, please try again');
            reject(new Error('Authentication timeout'));
        }, 300000);
    });
}
