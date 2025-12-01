// Antigravity authentication flow using management API

let antigravityAuthUrl = null;
let antigravityAuthState = null;
let antigravityPollingInterval = null;
let antigravityAbortController = null;
let antigravityPollingActive = false;

async function startAntigravityAuthFlow() {
    try {
        console.log('Starting Antigravity authentication flow...');
        await getAntigravityAuthUrl();
        showAntigravityAuthDialog();
    } catch (error) {
        console.error('Error starting Antigravity auth flow:', error);
        const msg = (error && (error.message || String(error))) || 'Unknown error';
        showError('Failed to start Antigravity authentication flow: ' + msg);
    }
}

async function getAntigravityAuthUrl() {
    try {
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

        // Add is_webui=true to let backend start web UI friendly callback forwarder
        const baseApiUrl = baseUrl.endsWith('/')
            ? `${baseUrl}v0/management/antigravity-auth-url`
            : `${baseUrl}/v0/management/antigravity-auth-url`;
        const apiUrl = `${baseApiUrl}?is_webui=true`;

        const headers = currentMode === 'local'
            ? { 'X-Management-Key': password, 'Content-Type': 'application/json' }
            : { 'Authorization': `Bearer ${password}`, 'Content-Type': 'application/json' };
        const response = await fetch(apiUrl, { method: 'GET', headers });
        if (!response.ok) throw new Error(`Failed to get Antigravity authentication URL: ${response.status}`);
        const data = await response.json();
        antigravityAuthUrl = data.url;
        antigravityAuthState = data.state;
        if (!antigravityAuthUrl) throw new Error('No valid authentication URL received');
        if (!antigravityAuthState) throw new Error('No valid authentication state received');
        console.log('Got Antigravity auth URL:', antigravityAuthUrl);
        console.log('Got Antigravity auth state:', antigravityAuthState);
    } catch (error) { console.error('Error getting Antigravity auth URL:', error); throw error; }
}

function showAntigravityAuthDialog() {
    const modal = document.createElement('div');
    modal.id = 'antigravity-auth-modal';
    modal.className = 'modal show';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Antigravity Authentication</h3>
                <button class="modal-close" id="antigravity-auth-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Please copy the link below and open it in your browser to complete Antigravity authentication:</p>
                    <div class="auth-url-container">
                        <input type="text" id="antigravity-auth-url-input" class="form-input" value="${antigravityAuthUrl}" readonly>
                        <button type="button" id="antigravity-copy-btn" class="copy-btn">Copy Link</button>
                    </div>
                    <div class="auth-status" id="antigravity-auth-status" style="display: none;">
                        <div class="auth-status-text">Waiting for authentication to complete...</div>
                        <div class="auth-status-spinner"></div>
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="antigravity-open-btn" class="btn-primary">Open Link</button>
                        <button type="button" id="antigravity-cancel-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);
    document.getElementById('antigravity-auth-modal-close').addEventListener('click', cancelAntigravityAuth);
    document.getElementById('antigravity-copy-btn').addEventListener('click', copyAntigravityUrl);
    document.getElementById('antigravity-open-btn').addEventListener('click', openAntigravityUrl);
    document.getElementById('antigravity-cancel-btn').addEventListener('click', cancelAntigravityAuth);
    document.addEventListener('keydown', handleAntigravityEscapeKey);
    const input = document.getElementById('antigravity-auth-url-input');
    input.focus(); input.select();

    // Start polling authentication status
    startAntigravityAuthPolling();
}

async function copyAntigravityUrl() {
    try { await navigator.clipboard.writeText(antigravityAuthUrl); showSuccessMessage('Link copied to clipboard'); }
    catch (error) { console.error('Error copying Antigravity URL:', error); showError('Failed to copy link: ' + error.message); }
}

function openAntigravityUrl() {
    try {
        if (window.__TAURI__?.shell?.open) { window.__TAURI__.shell.open(antigravityAuthUrl); }
        else { window.open(antigravityAuthUrl, '_blank'); }
        showSuccessMessage('Authentication link opened in browser');

        // Show polling status
        const statusDiv = document.getElementById('antigravity-auth-status');
        if (statusDiv) {
            statusDiv.style.display = 'block';
        }
    } catch (error) { console.error('Error opening Antigravity URL:', error); showError('Failed to open link: ' + error.message); }
}

// Start Antigravity authentication status polling
async function startAntigravityAuthPolling() {
    if (!antigravityAuthState) {
        console.error('No auth state available for polling');
        return;
    }

    try {
        console.log('Starting Antigravity authentication polling, state:', antigravityAuthState);
        antigravityPollingActive = true;
        await pollAntigravityAuthStatus(
            'Antigravity',
            antigravityAuthState,
            () => {
                console.log('Antigravity Authentication successful');
                showSuccessMessage('Antigravity authentication completed!');
                cancelAntigravityAuth();
                if (typeof loadAuthFiles === 'function') {
                    loadAuthFiles();
                }
            },
            (error) => {
                console.error('Antigravity Authentication failed:', error);
                showError('Antigravity Authentication failed: ' + error);
                cancelAntigravityAuth();
            }
        );
    } catch (error) {
        console.error('Antigravity Authentication polling error:', error);
        showError('Error occurred during Antigravity Authentication: ' + error.message);
        cancelAntigravityAuth();
    }
}

function cancelAntigravityAuth() {
    try {
        antigravityPollingActive = false;
        document.removeEventListener('keydown', handleAntigravityEscapeKey);
        const modal = document.getElementById('antigravity-auth-modal');
        if (modal) modal.remove();

        if (antigravityAbortController) {
            antigravityAbortController.abort();
            antigravityAbortController = null;
        }

        if (antigravityPollingInterval) {
            clearInterval(antigravityPollingInterval);
            antigravityPollingInterval = null;
        }

        antigravityAuthUrl = null;
        antigravityAuthState = null;
    } catch (error) {
        console.error('Error canceling Antigravity auth:', error);
    }
}

function handleAntigravityEscapeKey(e) { if (e.key === 'Escape') cancelAntigravityAuth(); }

// Antigravity authentication status polling function
async function pollAntigravityAuthStatus(authType, state, onSuccess, onError) {
    return new Promise((resolve, reject) => {
        antigravityAbortController = new AbortController();

        const pollInterval = setInterval(async () => {
            try {
                if (!antigravityPollingActive || antigravityAbortController.signal.aborted) {
                    clearInterval(pollInterval);
                    antigravityPollingInterval = null;
                    antigravityAbortController = null;
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
                    signal: antigravityAbortController.signal
                });

                if (!response.ok) {
                    throw new Error(`Failed to get authentication status: ${response.status}`);
                }

                const data = await response.json();
                console.log(`${authType} authentication status:`, data);

                if (data.status === 'ok') {
                    clearInterval(pollInterval);
                    antigravityPollingInterval = null;
                    antigravityAbortController = null;
                    onSuccess();
                    resolve(data);
                } else if (data.status === 'error') {
                    clearInterval(pollInterval);
                    antigravityPollingInterval = null;
                    const errMsg = data.error || 'Error occurred during authentication';
                    antigravityAbortController = null;
                    onError(errMsg);
                    reject(new Error(errMsg));
                }
            } catch (error) {
                if (error.name === 'AbortError') {
                    clearInterval(pollInterval);
                    antigravityPollingInterval = null;
                    antigravityAbortController = null;
                    return;
                }
                console.error(`Error polling ${authType} authentication status:`, error);
                clearInterval(pollInterval);
                antigravityPollingInterval = null;
                antigravityAbortController = null;
                onError(error.message);
                reject(error);
            }
        }, 2000);

        antigravityPollingInterval = pollInterval;
        console.log('Antigravity polling started, interval ID:', pollInterval);

        // Timeout after 5 minutes
        setTimeout(() => {
            clearInterval(pollInterval);
            antigravityPollingInterval = null;
            if (antigravityAbortController) {
                antigravityAbortController.abort();
                antigravityAbortController = null;
            }
            onError('Authentication timeout, please try again');
            reject(new Error('Authentication timeout'));
        }, 300000);
    });
}
