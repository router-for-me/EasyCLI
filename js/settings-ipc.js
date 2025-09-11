// Process state handling via Tauri events

function showProcessClosedError(message) {
    showError(message);
    setTimeout(() => {
        // Prefer Tauri backend command to open a proper login window
        if (typeof window !== 'undefined' && window.__TAURI__ && window.__TAURI__.core) {
            try { window.__TAURI__.core.invoke('open_login_window'); return; } catch (_) {}
        }
        // Fallback only if Tauri unavailable
        window.location.href = 'login.html';
    }, 3000);
}

if (window.__TAURI__?.event?.listen) {
    window.__TAURI__.event.listen('process-closed', (event) => {
        const data = event?.payload || {};
        console.log('CLIProxyAPI process closed:', data);
        showProcessClosedError(data.message || 'CLIProxyAPI process has closed');
    });

    window.__TAURI__.event.listen('process-exit-error', (event) => {
        const errorData = event?.payload || {};
        console.error('CLIProxyAPI process exited abnormally:', errorData);
        showProcessClosedError(`CLIProxyAPI process exited abnormally, exit code: ${errorData.code}`);
    });

    window.__TAURI__.event.listen('cliproxyapi-restarted', (event) => {
        const data = event?.payload || {};
        console.log('CLIProxyAPI process restarted successfully:', data);
        showSuccessMessage('CLIProxyAPI process restarted successfully!');
    });
}
