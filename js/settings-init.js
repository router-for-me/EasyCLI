// Page initialization after DOM is ready

document.addEventListener('DOMContentLoaded', async () => {
    try {
        const currentConfig = await getCurrentConfig();
        originalConfig = currentConfig;
        await initializeDebugSwitch();
        await initializePort();
        await initializeProxyUrl();
        await initializeRemoteManagement();
        await initializeAdditionalSettings();
        await initializeAutoStart();
        toggleLocalOnlyFields();
        updateServerStatus();
        updateActionButtons();

        const currentTabEl = document.querySelector('.tab.active');
        const currentTab = currentTabEl ? currentTabEl.getAttribute('data-tab') : 'basic';
        if (currentTab === 'access-token') {
            await loadAccessTokenKeys();
        } else if (currentTab === 'api') {
            await loadAllApiKeys();
        } else if (currentTab === 'openai') {
            await loadOpenaiProviders();
        }

        // Start keep-alive mechanism if in Local mode
        const currentType = localStorage.getItem('type') || 'local';
        if (currentType === 'local' && window.configManager) {
            window.configManager.startKeepAlive().catch(error => {
                console.error('Error starting keep-alive on settings init:', error);
            });
        }
    } catch (error) {
        console.error('Error initializing settings:', error);
        showError('Failed to load settings');
    }
});

// Stop keep-alive mechanism when page is unloaded
window.addEventListener('beforeunload', () => {
    if (window.configManager) {
        window.configManager.stopKeepAlive().catch(error => {
            console.error('Error stopping keep-alive on page unload:', error);
        });
    }
});

