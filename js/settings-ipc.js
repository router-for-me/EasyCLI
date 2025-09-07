// IPC event listeners for Electron environment and process state handling

function showProcessClosedError(message) {
    showError(message);
    setTimeout(() => {
        if (typeof require !== 'undefined') {
            const { ipcRenderer } = require('electron');
            ipcRenderer.send('return-to-login');
        } else {
            window.location.href = 'login.html';
        }
    }, 3000);
}

if (typeof require !== 'undefined') {
    const { ipcRenderer } = require('electron');

    ipcRenderer.on('process-closed', (event, data) => {
        console.log('CLIProxyAPI process closed:', data);
        showProcessClosedError(data.message);
    });

    ipcRenderer.on('process-exit-error', (event, errorData) => {
        console.error('CLIProxyAPI process exited abnormally:', errorData);
        showProcessClosedError(`CLIProxyAPI process exited abnormally, exit code: ${errorData.code}`);
    });

    ipcRenderer.on('cliproxyapi-restarted', (event, data) => {
        console.log('CLIProxyAPI process restarted successfully:', data);
        showSuccessMessage('CLIProxyAPI process restarted successfully!');
    });

    ipcRenderer.on('cliproxyapi-restart-failed', (event, errorData) => {
        console.error('CLIProxyAPI process restart failed:', errorData);
        showError(`CLIProxyAPI process restart failed: ${errorData.error}`);
    });
}

