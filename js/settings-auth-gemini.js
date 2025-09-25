// Gemini CLI authentication flow

let geminiLocalServer = null;
let geminiAuthUrl = null;
let geminiProjectId = null;
let geminiAuthState = null;
let geminiPollingInterval = null;
let geminiAbortController = null;
let geminiCancelled = false;
let geminiPollingActive = false;

function showGeminiProjectIdDialog() {
    const modal = document.createElement('div');
    modal.className = 'modal show';
    modal.id = 'gemini-project-modal';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Gemini CLI Authentication</h3>
                <button class="modal-close" id="gemini-project-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Please enter Google Cloud Project ID (optional):</p>
                    <div class="form-group">
                        <input type="text" id="gemini-project-id-input" class="form-input" placeholder="Enter Project ID (optional)">
                        <small class="form-help">If no Project ID is entered, the default project will be used</small>
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="gemini-project-confirm-btn" class="btn-primary">Confirm</button>
                        <button type="button" id="gemini-project-cancel-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);
    document.getElementById('gemini-project-modal-close').addEventListener('click', cancelGeminiProjectDialogAndReset);
    document.getElementById('gemini-project-confirm-btn').addEventListener('click', confirmGeminiProjectId);
    document.getElementById('gemini-project-cancel-btn').addEventListener('click', cancelGeminiProjectDialogAndReset);
    // Disable backdrop click-to-close to avoid accidental dismiss
    document.addEventListener('keydown', handleGeminiProjectEscapeKey);
    document.getElementById('gemini-project-id-input').focus();
}

async function confirmGeminiProjectId() {
    try {
        const projectIdInput = document.getElementById('gemini-project-id-input');
        geminiProjectId = projectIdInput.value.trim();
        console.log('Gemini Project ID set to:', geminiProjectId || '(empty)');
        cancelGeminiProjectDialog();
        await startGeminiAuthFlow();
    } catch (error) {
        console.error('Error confirming Gemini project ID:', error);
        showError('Failed to start Gemini CLI Authentication flow: ' + error.message);
    }
}

function cancelGeminiProjectDialog() {
    document.removeEventListener('keydown', handleGeminiProjectEscapeKey);
    const modal = document.getElementById('gemini-project-modal');
    if (modal) modal.remove();
}

function cancelGeminiProjectDialogAndReset() {
    document.removeEventListener('keydown', handleGeminiProjectEscapeKey);
    const modal = document.getElementById('gemini-project-modal');
    if (modal) modal.remove();
    geminiProjectId = null;
}

function handleGeminiProjectEscapeKey(e) { if (e.key === 'Escape') cancelGeminiProjectDialogAndReset(); }

async function startGeminiAuthFlow() {
    try {
        console.log('Starting Gemini CLI authentication flow...', geminiProjectId ? `Project ID: ${geminiProjectId}` : 'No Project ID');

        // Gemini CLI authentication requires starting HTTP server in both Local and Remote modes
        await startGeminiLocalServer();

        await getGeminiAuthUrl();
        showGeminiAuthDialog();
    } catch (error) {
        console.error('Error starting Gemini auth flow:', error);
        const msg = (error && (error.message || String(error))) || 'Unknown error';
        showError('Failed to start Gemini CLI Authentication flow: ' + msg);
        if (geminiLocalServer) { await stopGeminiLocalServer(); }
    }
}

async function startGeminiLocalServer() {
    try {
        const currentMode = localStorage.getItem('type') || 'local';
        let localPort = null, baseUrl = null;
        if (currentMode === 'local') {
            const config = await configManager.getConfig();
            localPort = config.port || 8317;
        } else {
            configManager.refreshConnection();
            baseUrl = configManager.baseUrl;
            if (!baseUrl) throw new Error('Missing base-url configuration');
        }
        await window.__TAURI__.core.invoke('start_callback_server', {
            provider: 'google',
            listenPort: 8085,
            mode: currentMode,
            baseUrl: baseUrl,
            localPort: localPort
        });
    } catch (error) { throw error; }
}

async function handleGeminiCallback(req, res) {
    try {
        console.log('Received callback from Gemini:', req.url);
        const url = new URL(req.url, `http://localhost:8085`);
        const currentMode = localStorage.getItem('type') || 'local';

        let redirectUrl;
        if (currentMode === 'local') {
            // Local mode: redirect to http://127.0.0.1:{port}/google/callback
            const config = await configManager.getConfig();
            const port = config.port || 8317; // Default port
            redirectUrl = `http://127.0.0.1:${port}/google/callback${url.search}`;
        } else {
            // Remote mode: redirect to base-url/google/callback
            configManager.refreshConnection();
            const baseUrl = configManager.baseUrl;
            if (!baseUrl) {
                res.writeHead(400, { 'Content-Type': 'text/plain' });
                res.end('Missing base-url configuration');
                return;
            }
            redirectUrl = baseUrl.endsWith('/') ? `${baseUrl}google/callback${url.search}` : `${baseUrl}/google/callback${url.search}`;
        }

        console.log('Redirecting to:', redirectUrl);
        res.writeHead(302, { 'Location': redirectUrl });
        res.end();
        setTimeout(async () => { await stopGeminiLocalServer(); showSuccessMessage('Gemini CLI Authentication completed!'); }, 1000);
    } catch (error) {
        console.error('Error handling Gemini callback:', error);
        res.writeHead(500, { 'Content-Type': 'text/plain' });
        res.end('Internal server error');
    }
}

async function stopGeminiLocalServer() { try { await window.__TAURI__.core.invoke('stop_callback_server', { listenPort: 8085 }); } catch (_) { } }

async function getGeminiAuthUrl() {
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

        let apiUrl = baseUrl.endsWith('/') ? `${baseUrl}v0/management/gemini-cli-auth-url` : `${baseUrl}/v0/management/gemini-cli-auth-url`;

        // If there is a project-id, add it as a GET parameter to the URL
        if (geminiProjectId) {
            const separator = apiUrl.includes('?') ? '&' : '?';
            apiUrl += `${separator}project_id=${encodeURIComponent(geminiProjectId)}`;
        }

        const headers = currentMode === 'local'
            ? { 'X-Management-Key': password, 'Content-Type': 'application/json' }
            : { 'Authorization': `Bearer ${password}`, 'Content-Type': 'application/json' };
        const response = await fetch(apiUrl, { method: 'GET', headers });
        if (!response.ok) throw new Error(`Failed to get Gemini authentication URL: ${response.status}`);
        const data = await response.json();
        geminiAuthUrl = data.url;
        geminiAuthState = data.state;
        if (!geminiAuthUrl) throw new Error('No valid authentication URL received');
        if (!geminiAuthState) throw new Error('No valid authentication state received');
        console.log('Got Gemini auth URL:', geminiAuthUrl);
        console.log('Got Gemini auth state:', geminiAuthState);
    } catch (error) { console.error('Error getting Gemini auth URL:', error); throw error; }
}

function showGeminiAuthDialog() {
    const modal = document.createElement('div');
    modal.className = 'modal show';
    modal.id = 'gemini-auth-modal';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Gemini CLI Authentication</h3>
                <button class="modal-close" id="gemini-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Please copy the link below and open it in your browser, or click the "Open Link" button directly:</p>
                    <div class="auth-url-container">
                        <input type="text" id="gemini-auth-url-input" class="form-input" value="${geminiAuthUrl}" readonly>
                        <button type="button" id="copy-gemini-url-btn" class="copy-btn">Copy Link</button>
                    </div>
                    <div class="auth-status" id="gemini-auth-status" style="display: none;">
                        <div class="auth-status-text">Waiting for authentication to complete...</div>
                        <div class="auth-status-spinner"></div>
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="open-gemini-url-btn" class="btn-primary">Open Link</button>
                        <button type="button" id="cancel-gemini-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);
    document.getElementById('gemini-modal-close').addEventListener('click', cancelGeminiAuth);
    document.getElementById('copy-gemini-url-btn').addEventListener('click', copyGeminiUrl);
    document.getElementById('open-gemini-url-btn').addEventListener('click', openGeminiUrl);
    document.getElementById('cancel-gemini-btn').addEventListener('click', cancelGeminiAuth);
    // Disable backdrop click-to-close to avoid accidental dismiss
    document.addEventListener('keydown', handleGeminiEscapeKey);
    const input = document.getElementById('gemini-auth-url-input');
    input.focus(); input.select();

    // Start polling authentication status
    startGeminiAuthPolling();
}

async function copyGeminiUrl() {
    try { const urlInput = document.getElementById('gemini-auth-url-input'); await navigator.clipboard.writeText(urlInput.value); showSuccessMessage('Link copied to clipboard'); }
    catch (error) { console.error('Error copying URL:', error); showError('Failed to copy link'); }
}

function openGeminiUrl() {
    try {
        if (window.__TAURI__?.shell?.open) { window.__TAURI__.shell.open(geminiAuthUrl); }
        else { window.open(geminiAuthUrl, '_blank'); }
        showSuccessMessage('Authentication link opened in browser');

        // Show polling status
        const statusDiv = document.getElementById('gemini-auth-status');
        if (statusDiv) {
            statusDiv.style.display = 'block';
        }
    } catch (error) { console.error('Error opening URL:', error); showError('Failed to open link'); }
}

// Start Gemini authentication status polling
async function startGeminiAuthPolling() {
    if (!geminiAuthState) {
        console.error('No auth state available for polling');
        return;
    }

    try {
        console.log('=== Starting GEMINI authentication polling ===, state:', geminiAuthState);
        geminiCancelled = false; // Reset cancel flag
        await pollGeminiAuthStatus(
            'Gemini CLI',
            geminiAuthState,
            () => {
                // Authentication successful
                console.log('Gemini CLI Authentication successful');
                showSuccessMessage('Gemini CLI Authentication completed!');
                cancelGeminiAuth();
                // Refresh auth files list
                if (typeof loadAuthFiles === 'function') {
                    loadAuthFiles();
                }
            },
            (error) => {
                // Authentication failed
                console.error('Gemini CLI Authentication failed:', error);
                showError('Gemini CLI Authentication failed: ' + error);
                cancelGeminiAuth();
            }
        );
    } catch (error) {
        console.error('Gemini CLI Authentication polling error:', error);
        showError('Error occurred during Gemini CLI Authentication: ' + error.message);
        cancelGeminiAuth();
    }
}

async function cancelGeminiAuth() {
    try {
        console.log('=== Cancel GEMINI authentication ===, current polling interval ID:', geminiPollingInterval);

        // Set cancel flag
        geminiCancelled = true;
        geminiPollingActive = false;

        document.removeEventListener('keydown', handleGeminiEscapeKey);
        const modal = document.getElementById('gemini-auth-modal');
        if (modal) modal.remove();
        // Always stop local callback server to free the port
        await stopGeminiLocalServer();

        // Cancel ongoing requests
        if (geminiAbortController) {
            console.log('Canceling Gemini polling request');
            geminiAbortController.abort();
            geminiAbortController = null;
        }

        if (geminiPollingInterval) {
            console.log('Stopping Gemini polling, interval ID:', geminiPollingInterval);
            clearInterval(geminiPollingInterval);
            geminiPollingInterval = null;
            console.log('Gemini polling stopped');
        } else {
            console.log('No active Gemini polling to stop');
        }
        geminiAuthUrl = null;
        geminiAuthState = null;
        geminiProjectId = null;
    } catch (error) { console.error('Error canceling Gemini auth:', error); }
}

function handleGeminiEscapeKey(e) { if (e.key === 'Escape') cancelGeminiAuth(); }

// Gemini authentication status polling function
async function pollGeminiAuthStatus(authType, state, onSuccess, onError) {
    return new Promise((resolve, reject) => {
        // Create AbortController for canceling requests
        geminiAbortController = new AbortController();

        const pollInterval = setInterval(async () => {
            try {
                console.log('=== GEMINI polling is running ===', new Date().toLocaleTimeString(), 'Active status:', geminiPollingActive);

                // Check if already canceled
                if (!geminiPollingActive || geminiCancelled || geminiAbortController.signal.aborted) {
                    console.log('Gemini polling has been canceled, stopping polling');
                    clearInterval(pollInterval);
                    geminiPollingInterval = null;
                    geminiPollingActive = false;
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
                    signal: geminiAbortController.signal
                });

                if (!response.ok) {
                    throw new Error(`Failed to get authentication status: ${response.status}`);
                }

                const data = await response.json();
                console.log(`${authType} authentication status:`, data);

                if (data.status === 'ok') {
                    clearInterval(pollInterval);
                    geminiPollingInterval = null;
                    geminiAbortController = null;
                    geminiPollingActive = false;
                    onSuccess();
                    resolve(data);
                } else if (data.status === 'error') {
                    clearInterval(pollInterval);
                    geminiPollingInterval = null;
                    geminiAbortController = null;
                    geminiPollingActive = false;
                    onError(data.error || 'Error occurred during authentication');
                    reject(new Error(data.error || 'Error occurred during authentication'));
                }
                // If status is 'wait', continue polling
            } catch (error) {
                if (error.name === 'AbortError') {
                    console.log('Gemini polling request canceled');
                    clearInterval(pollInterval);
                    geminiPollingInterval = null;
                    geminiAbortController = null;
                    geminiPollingActive = false;
                    return;
                }
                console.error(`Error polling ${authType} authentication status:`, error);
                clearInterval(pollInterval);
                geminiPollingInterval = null;
                geminiAbortController = null;
                geminiPollingActive = false;
                onError(error.message);
                reject(error);
            }
        }, 2000); // Poll every 2 seconds

        // Store polling interval ID in global variable for cancellation
        geminiPollingInterval = pollInterval;
        geminiPollingActive = true;
        console.log('=== GEMINI polling started ===，interval ID:', pollInterval, 'Active status:', geminiPollingActive);

        // Set timeout (5 minutes)
        setTimeout(() => {
            clearInterval(pollInterval);
            geminiPollingInterval = null;
            geminiPollingActive = false;
            if (geminiAbortController) {
                geminiAbortController.abort();
                geminiAbortController = null;
            }
            onError('Authentication timeout, please try again');
            reject(new Error('Authentication timeout'));
        }, 300000);
    });
}
