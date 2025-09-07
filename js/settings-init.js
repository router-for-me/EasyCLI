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
        toggleLocalOnlyFields();
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
    } catch (error) {
        console.error('Error initializing settings:', error);
        showError('Failed to load settings');
    }
});

