// Claude Code authentication flow

let claudeLocalServer = null;
let claudeAuthUrl = null;
let claudeAuthState = null;
let claudePollingInterval = null;
let claudeAbortController = null;
let claudeCancelled = false;
let claudePollingActive = false;

async function startClaudeAuthFlow() {
    try {
        console.log('Starting Claude Code authentication flow...');

        // Claude Code authentication requires starting HTTP server in both Local and Remote modes
        await startClaudeLocalServer();

        await getClaudeAuthUrl();
        showClaudeAuthDialog();
    } catch (error) {
        console.error('Error starting Claude auth flow:', error);
        const msg = (error && (error.message || String(error))) || 'Unknown error';
        showError('Failed to start Claude Code authentication flow: ' + msg);
        if (claudeLocalServer) {
            await stopClaudeLocalServer();
        }
    }
}

async function startClaudeLocalServer() {
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
            provider: 'anthropic',
            listenPort: 54545,
            mode: currentMode,
            baseUrl: baseUrl,
            localPort: localPort
        });
    } catch (error) { throw error; }
}

async function handleClaudeCallback(req, res) {
    try {
        console.log('Received callback from Claude:', req.url);
        const url = new URL(req.url, `http://localhost:54545`);
        const currentMode = localStorage.getItem('type') || 'local';

        let redirectUrl;
        if (currentMode === 'local') {
            // Local mode: redirect to http://127.0.0.1:{port}/anthropic/callback
            const config = await configManager.getConfig();
            const port = config.port || 8317; // Default port
            redirectUrl = `http://127.0.0.1:${port}/anthropic/callback${url.search}`;
        } else {
            // Remote mode: redirect to base-url/anthropic/callback
            const baseUrl = localStorage.getItem('base-url');
            if (!baseUrl) {
                res.writeHead(400, { 'Content-Type': 'text/plain' });
                res.end('Missing base-url configuration');
                return;
            }
            redirectUrl = baseUrl.endsWith('/') ? `${baseUrl}anthropic/callback${url.search}` : `${baseUrl}/anthropic/callback${url.search}`;
        }

        console.log('Redirecting to:', redirectUrl);
        res.writeHead(302, { 'Location': redirectUrl });
        res.end();
        setTimeout(async () => { await stopClaudeLocalServer(); showSuccessMessage('Claude Code authentication completed!'); }, 1000);
    } catch (error) {
        console.error('Error handling Claude callback:', error);
        res.writeHead(500, { 'Content-Type': 'text/plain' });
        res.end('Internal server error');
    }
}

async function stopClaudeLocalServer() { try { await window.__TAURI__.core.invoke('stop_callback_server', { listenPort: 54545 }); } catch (_) { } }

async function getClaudeAuthUrl() {
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

        const apiUrl = baseUrl.endsWith('/') ? `${baseUrl}v0/management/anthropic-auth-url` : `${baseUrl}/v0/management/anthropic-auth-url`;
        const headers = currentMode === 'local'
            ? { 'X-Management-Key': password, 'Content-Type': 'application/json' }
            : { 'Authorization': `Bearer ${password}`, 'Content-Type': 'application/json' };
        const response = await fetch(apiUrl, { method: 'GET', headers: headers });
        if (!response.ok) throw new Error(`Failed to get Claude authentication URL: ${response.status}`);
        const data = await response.json();
        claudeAuthUrl = data.url;
        claudeAuthState = data.state;
        if (!claudeAuthUrl) throw new Error('No valid authentication URL received');
        if (!claudeAuthState) throw new Error('No valid authentication state received');
        console.log('Got Claude auth URL:', claudeAuthUrl);
        console.log('Got Claude auth state:', claudeAuthState);
    } catch (error) { console.error('Error getting Claude auth URL:', error); throw error; }
}

function showClaudeAuthDialog() {
    const modal = document.createElement('div');
    modal.className = 'modal show';
    modal.id = 'claude-auth-modal';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Claude Code Authentication</h3>
                <button class="modal-close" id="claude-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Please copy the link below and open it in your browser, or click the "Open Link" button directly:</p>
                    <div class="auth-url-container">
                        <input type="text" id="claude-auth-url-input" class="form-input" value="${claudeAuthUrl}" readonly>
                        <button type="button" id="copy-claude-url-btn" class="copy-btn">Copy Link</button>
                    </div>
                    <div class="auth-status" id="claude-auth-status" style="display: none;">
                        <div class="auth-status-text">Waiting for authentication to complete...</div>
                        <div class="auth-status-spinner"></div>
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="open-claude-url-btn" class="btn-primary">Open Link</button>
                        <button type="button" id="cancel-claude-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);
    document.getElementById('claude-modal-close').addEventListener('click', cancelClaudeAuth);
    document.getElementById('copy-claude-url-btn').addEventListener('click', copyClaudeUrl);
    document.getElementById('open-claude-url-btn').addEventListener('click', openClaudeUrl);
    document.getElementById('cancel-claude-btn').addEventListener('click', cancelClaudeAuth);
    // Disable backdrop click-to-close to avoid accidental dismiss
    document.addEventListener('keydown', handleClaudeEscapeKey);

    // Start polling authentication status
    startClaudeAuthPolling();
}

async function copyClaudeUrl() {
    try { const urlInput = document.getElementById('claude-auth-url-input'); await navigator.clipboard.writeText(urlInput.value); showSuccessMessage('Link copied to clipboard'); }
    catch (error) { console.error('Error copying URL:', error); showError('Failed to copy link'); }
}

function openClaudeUrl() {
    try {
        if (window.__TAURI__?.shell?.open) { window.__TAURI__.shell.open(claudeAuthUrl); }
        else { window.open(claudeAuthUrl, '_blank'); }
        showSuccessMessage('Authentication link opened in browser');

        // Show polling status
        const statusDiv = document.getElementById('claude-auth-status');
        if (statusDiv) {
            statusDiv.style.display = 'block';
        }
    } catch (error) { console.error('Error opening URL:', error); showError('Failed to open link'); }
}

// Start Claude authentication status polling
async function startClaudeAuthPolling() {
    if (!claudeAuthState) {
        console.error('No auth state available for polling');
        return;
    }

    try {
        console.log('Starting Claude authentication polling, state:', claudeAuthState);
        claudeCancelled = false; // Reset cancel flag
        await pollClaudeAuthStatus(
            'Claude Code',
            claudeAuthState,
            () => {
                // Authentication successful
                console.log('Claude Code Authentication successful');
                showSuccessMessage('Claude Code authentication completed!');
                cancelClaudeAuth();
                // Refresh auth files list
                if (typeof loadAuthFiles === 'function') {
                    loadAuthFiles();
                }
            },
            (error) => {
                // Authentication failed
                console.error('Claude Code Authentication failed:', error);
                showError('Claude Code Authentication failed: ' + error);
                cancelClaudeAuth();
            }
        );
    } catch (error) {
        console.error('Claude Code Authentication polling error:', error);
        showError('Error occurred during Claude Code Authentication: ' + error.message);
        cancelClaudeAuth();
    }
}

async function cancelClaudeAuth() {
    try {
        console.log('Canceling Claude authentication, current polling interval ID:', claudePollingInterval);

        // Set cancel flag
        claudeCancelled = true;
        claudePollingActive = false;

        document.removeEventListener('keydown', handleClaudeEscapeKey);
        const modal = document.getElementById('claude-auth-modal');
        if (modal) modal.remove();
        // Always stop local callback server to free the port
        await stopClaudeLocalServer();

        // Cancel ongoing requests
        if (claudeAbortController) {
            console.log('Canceling Claude polling request');
            claudeAbortController.abort();
            claudeAbortController = null;
        }

        if (claudePollingInterval) {
            console.log('Stopping Claude polling, interval ID:', claudePollingInterval);
            clearInterval(claudePollingInterval);
            claudePollingInterval = null;
            console.log('Claude polling stopped');
        } else {
            console.log('No active Claude polling to stop');
        }
        claudeAuthUrl = null;
        claudeAuthState = null;
    } catch (error) { console.error('Error canceling Claude auth:', error); }
}

function handleClaudeEscapeKey(e) { if (e.key === 'Escape') cancelClaudeAuth(); }

// Claude authentication status polling function
async function pollClaudeAuthStatus(authType, state, onSuccess, onError) {
    return new Promise((resolve, reject) => {
        // Create AbortController for canceling requests
        claudeAbortController = new AbortController();

        const pollInterval = setInterval(async () => {
            try {
                console.log('Claude polling is running...', new Date().toLocaleTimeString(), 'Active status:', claudePollingActive);

                // Check if already canceled
                if (!claudePollingActive || claudeCancelled || claudeAbortController.signal.aborted) {
                    console.log('Claude polling has been canceled, stopping polling');
                    clearInterval(pollInterval);
                    claudePollingInterval = null;
                    claudePollingActive = false;
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
                    signal: claudeAbortController.signal
                });

                if (!response.ok) {
                    throw new Error(`Failed to get authentication status: ${response.status}`);
                }

                const data = await response.json();
                console.log(`${authType} authentication status:`, data);

                if (data.status === 'ok') {
                    clearInterval(pollInterval);
                    claudePollingInterval = null;
                    claudeAbortController = null;
                    claudePollingActive = false;
                    onSuccess();
                    resolve(data);
                } else if (data.status === 'error') {
                    clearInterval(pollInterval);
                    claudePollingInterval = null;
                    claudeAbortController = null;
                    claudePollingActive = false;
                    onError(data.error || 'Error occurred during authentication');
                    reject(new Error(data.error || 'Error occurred during authentication'));
                }
                // If status is 'wait', continue polling
            } catch (error) {
                if (error.name === 'AbortError') {
                    console.log('Claude polling request canceled');
                    clearInterval(pollInterval);
                    claudePollingInterval = null;
                    claudeAbortController = null;
                    claudePollingActive = false;
                    return;
                }
                console.error(`Error polling ${authType} authentication status:`, error);
                clearInterval(pollInterval);
                claudePollingInterval = null;
                claudeAbortController = null;
                claudePollingActive = false;
                onError(error.message);
                reject(error);
            }
        }, 2000); // Poll every 2 seconds

        // Store polling interval ID in global variable for cancellation
        claudePollingInterval = pollInterval;
        claudePollingActive = true;
        console.log('Claude polling started, interval ID:', pollInterval, 'Active status:', claudePollingActive);

        // Set timeout (5 minutes)
        setTimeout(() => {
            clearInterval(pollInterval);
            claudePollingInterval = null;
            claudePollingActive = false;
            if (claudeAbortController) {
                claudeAbortController.abort();
                claudeAbortController = null;
            }
            onError('Authentication timeout, please try again');
            reject(new Error('Authentication timeout'));
        }, 300000);
    });
}
