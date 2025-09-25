// Codex authentication flow (Electron-based local callback server)

let codexLocalServer = null;
let codexAuthUrl = null;
let codexAuthState = null;
let codexPollingInterval = null;
let codexAbortController = null;

async function startCodexAuthFlow() {
    try {
        console.log('Starting Codex authentication flow...');

        // Codex authentication requires starting HTTP server in both Local and Remote modes
        await startCodexLocalServer();

        await getCodexAuthUrl();
        showCodexAuthDialog();
    } catch (error) {
        console.error('Error starting Codex auth flow:', error);
        const msg = (error && (error.message || String(error))) || 'Unknown error';
        showError('Failed to start Codex authentication flow: ' + msg);
        if (codexLocalServer) {
            await stopCodexLocalServer();
        }
    }
}

async function startCodexLocalServer() {
    try {
        const currentMode = localStorage.getItem('type') || 'local';
        let localPort = null, baseUrl = null;
        if (currentMode === 'local') {
            const config = await configManager.getConfig();
            localPort = config.port || 8317;
        } else {
            baseUrl = localStorage.getItem('base-url');
            if (!baseUrl) throw new Error('Missing base-url configuration');
        }
        await window.__TAURI__.core.invoke('start_callback_server', {
            provider: 'codex',
            listenPort: 1455,
            mode: currentMode,
            baseUrl: baseUrl,
            localPort: localPort
        });
    } catch (error) { throw error; }
}

async function handleCodexCallback(req, res) {
    try {
        console.log('Received callback from Codex:', req.url);
        const url = new URL(req.url, `http://localhost:1455`);
        const currentMode = localStorage.getItem('type') || 'local';

        let redirectUrl;
        if (currentMode === 'local') {
            // Local mode: redirect to http://127.0.0.1:{port}/codex/callback
            const config = await configManager.getConfig();
            const port = config.port || 8317; // Default port
            redirectUrl = `http://127.0.0.1:${port}/codex/callback${url.search}`;
        } else {
            // Remote mode: redirect to base-url/codex/callback
            const baseUrl = localStorage.getItem('base-url');
            if (!baseUrl) {
                res.writeHead(400, { 'Content-Type': 'text/plain' });
                res.end('Missing base-url configuration');
                return;
            }
            redirectUrl = baseUrl.endsWith('/') ? `${baseUrl}codex/callback${url.search}` : `${baseUrl}/codex/callback${url.search}`;
        }

        console.log('Redirecting to:', redirectUrl);
        res.writeHead(302, { 'Location': redirectUrl });
        res.end();
        setTimeout(async () => { await stopCodexLocalServer(); showSuccessMessage('Codex authentication completed!'); }, 1000);
    } catch (error) {
        console.error('Error handling Codex callback:', error);
        res.writeHead(500, { 'Content-Type': 'text/plain' });
        res.end('Internal server error');
    }
}

async function stopCodexLocalServer() { try { await window.__TAURI__.core.invoke('stop_callback_server', { listenPort: 1455 }); } catch (_) { } }

async function getCodexAuthUrl() {
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
            // Read configuration from localStorage in Remote mode
            baseUrl = localStorage.getItem('base-url');
            password = localStorage.getItem('password');
            if (!baseUrl || !password) throw new Error('Missing connection information');
        }

        const apiUrl = baseUrl.endsWith('/') ? `${baseUrl}v0/management/codex-auth-url` : `${baseUrl}/v0/management/codex-auth-url`;
        const headers = currentMode === 'local'
            ? { 'X-Management-Key': password, 'Content-Type': 'application/json' }
            : { 'Authorization': `Bearer ${password}`, 'Content-Type': 'application/json' };
        const response = await fetch(apiUrl, { method: 'GET', headers: headers });
        if (!response.ok) throw new Error(`Failed to get Codex authentication URL: ${response.status}`);
        const data = await response.json();
        codexAuthUrl = data.url;
        codexAuthState = data.state;
        if (!codexAuthUrl) throw new Error('No valid authentication URL received');
        if (!codexAuthState) throw new Error('No valid authentication state received');
        console.log('Got Codex auth URL:', codexAuthUrl);
        console.log('Got Codex auth state:', codexAuthState);
    } catch (error) { console.error('Error getting Codex auth URL:', error); throw error; }
}

function showCodexAuthDialog() {
    const modal = document.createElement('div');
    modal.className = 'modal show';
    modal.id = 'codex-auth-modal';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Codex Authentication</h3>
                <button class="modal-close" id="codex-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Please copy the link below and open it in your browser, or click the "Open Link" button directly:</p>
                    <div class="auth-url-container">
                        <input type="text" id="codex-auth-url-input" class="form-input" value="${codexAuthUrl}" readonly>
                        <button type="button" id="codex-copy-btn" class="copy-btn">Copy Link</button>
                    </div>
                    <div class="auth-status" id="codex-auth-status" style="display: none;">
                        <div class="auth-status-text">Waiting for authentication to complete...</div>
                        <div class="auth-status-spinner"></div>
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="codex-open-btn" class="btn-primary">Open Link</button>
                        <button type="button" id="codex-cancel-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);
    document.getElementById('codex-modal-close').addEventListener('click', cancelCodexAuth);
    document.getElementById('codex-copy-btn').addEventListener('click', copyCodexUrl);
    document.getElementById('codex-open-btn').addEventListener('click', openCodexUrl);
    document.getElementById('codex-cancel-btn').addEventListener('click', cancelCodexAuth);
    // Disable backdrop click-to-close to avoid accidental dismiss
    document.addEventListener('keydown', handleCodexEscapeKey);
    const input = document.getElementById('codex-auth-url-input');
    input.focus(); input.select();

    // Start polling authentication status
    startCodexAuthPolling();
}

async function copyCodexUrl() {
    try { await navigator.clipboard.writeText(codexAuthUrl); showSuccessMessage('Link copied to clipboard'); }
    catch (error) { console.error('Error copying Codex URL:', error); showError('Failed to copy link: ' + error.message); }
}

function openCodexUrl() {
    try {
        if (window.__TAURI__?.shell?.open) { window.__TAURI__.shell.open(codexAuthUrl); }
        else { window.open(codexAuthUrl, '_blank'); }
        showSuccessMessage('Authentication link opened in browser');

        // Show polling status
        const statusDiv = document.getElementById('codex-auth-status');
        if (statusDiv) {
            statusDiv.style.display = 'block';
        }
    } catch (error) { console.error('Error opening Codex URL:', error); showError('Failed to open link: ' + error.message); }
}

// Start Codex authentication status polling
async function startCodexAuthPolling() {
    if (!codexAuthState) {
        console.error('No auth state available for polling');
        return;
    }

    try {
        await pollCodexAuthStatus(
            'Codex',
            codexAuthState,
            () => {
                // Authentication successful
                console.log('Codex Authentication successful');
                showSuccessMessage('Codex authentication completed!');
                cancelCodexAuth();
                // Refresh auth files list
                if (typeof loadAuthFiles === 'function') {
                    loadAuthFiles();
                }
            },
            (error) => {
                // Authentication failed
                console.error('Codex Authentication failed:', error);
                showError('Codex Authentication failed: ' + error);
                cancelCodexAuth();
            }
        );
    } catch (error) {
        console.error('Codex Authentication polling error:', error);
        showError('Error occurred during Codex Authentication: ' + error.message);
        cancelCodexAuth();
    }
}

async function cancelCodexAuth() {
    try {
        console.log('Canceling Codex authentication, current polling interval ID:', codexPollingInterval);
        document.removeEventListener('keydown', handleCodexEscapeKey);
        const modal = document.getElementById('codex-auth-modal');
        if (modal) modal.remove();
        // Always stop local callback server to free the port
        await stopCodexLocalServer();

        // Cancel ongoing requests
        if (codexAbortController) {
            console.log('Canceling Codex polling request');
            codexAbortController.abort();
            codexAbortController = null;
        }

        if (codexPollingInterval) {
            console.log('Stopping Codex polling, interval ID:', codexPollingInterval);
            clearInterval(codexPollingInterval);
            codexPollingInterval = null;
            console.log('Codex polling stopped');
        } else {
            console.log('No active Codex polling to stop');
        }
        codexAuthUrl = null;
        codexAuthState = null;
    } catch (error) { console.error('Error canceling Codex auth:', error); }
}

function handleCodexEscapeKey(e) { if (e.key === 'Escape') cancelCodexAuth(); }

// Codex authentication status polling function
async function pollCodexAuthStatus(authType, state, onSuccess, onError) {
    return new Promise((resolve, reject) => {
        // Create AbortController for canceling requests
        codexAbortController = new AbortController();

        const pollInterval = setInterval(async () => {
            try {
                // Check if already canceled
                if (codexAbortController.signal.aborted) {
                    console.log('Codex polling has been canceled, stopping polling');
                    clearInterval(pollInterval);
                    codexPollingInterval = null;
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
                    baseUrl = localStorage.getItem('base-url');
                    password = localStorage.getItem('password');
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
                    signal: codexAbortController.signal
                });

                if (!response.ok) {
                    throw new Error(`Failed to get authentication status: ${response.status}`);
                }

                const data = await response.json();
                console.log(`${authType} authentication status:`, data);

                if (data.status === 'ok') {
                    clearInterval(pollInterval);
                    codexPollingInterval = null;
                    codexAbortController = null;
                    onSuccess();
                    resolve(data);
                } else if (data.status === 'error') {
                    clearInterval(pollInterval);
                    codexPollingInterval = null;
                    codexAbortController = null;
                    onError(data.error || 'Error occurred during authentication');
                    reject(new Error(data.error || 'Error occurred during authentication'));
                }
                // If status is 'wait', continue polling
            } catch (error) {
                if (error.name === 'AbortError') {
                    console.log('Codex polling request canceled');
                    clearInterval(pollInterval);
                    codexPollingInterval = null;
                    codexAbortController = null;
                    return;
                }
                console.error(`Error polling ${authType} authentication status:`, error);
                clearInterval(pollInterval);
                codexPollingInterval = null;
                codexAbortController = null;
                onError(error.message);
                reject(error);
            }
        }, 2000); // Poll every 2 seconds

        // Store polling interval ID in global variable for cancellation
        codexPollingInterval = pollInterval;
        console.log('Codex polling started, interval ID:', pollInterval);

        // Set timeout (5 minutes)
        setTimeout(() => {
            clearInterval(pollInterval);
            codexPollingInterval = null;
            if (codexAbortController) {
                codexAbortController.abort();
                codexAbortController = null;
            }
            onError('Authentication timeout, please try again');
            reject(new Error('Authentication timeout'));
        }, 300000);
    });
}
