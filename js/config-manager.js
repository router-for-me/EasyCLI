/**
 * Configuration Manager Abstraction Layer
 * Unified operation interface for Local and Remote modes
 */
class ConfigManager {
    constructor() {
        this.type = localStorage.getItem('type') || 'local';
        this.baseUrl = localStorage.getItem('base-url');
        this.password = localStorage.getItem('password');
        this.keepAliveInterval = null;
        this.keepAliveEnabled = false;
    }

    /**
     * Save multiple files as a ZIP with a Save As dialog when possible
     * @param {Array<{name:string, content:string|Uint8Array|ArrayBuffer}>} files
     * @param {string} suggestedName
     * @returns {Promise<Object>} result
     */
    async saveFilesAsZip(files, suggestedName = 'auth-files.zip') {
        try {
            if (!Array.isArray(files) || files.length === 0) {
                return { success: false, error: 'No files to save' };
            }
            if (typeof window.__zipFiles !== 'function') {
                // Missing ZIP util
                return { success: false, error: 'ZIP utility not loaded' };
            }
            const blob = window.__zipFiles(files);
            if (typeof window.showSaveFilePicker === 'function') {
                try {
                    const handle = await window.showSaveFilePicker({
                        suggestedName,
                        types: [{ description: 'ZIP archive', accept: { 'application/zip': ['.zip'] } }]
                    });
                    const writable = await handle.createWritable();
                    await writable.write(blob);
                    await writable.close();
                } catch (e) {
                    if (e && e.name === 'AbortError') {
                        return { success: false, error: 'User cancelled save dialog' };
                    }
                    // Fallback to anchor download
                    const url = URL.createObjectURL(blob);
                    const a = document.createElement('a');
                    a.href = url; a.download = suggestedName;
                    document.body.appendChild(a); a.click(); document.body.removeChild(a);
                    URL.revokeObjectURL(url);
                }
            } else {
                // Fallback: anchor download (default Downloads folder or per-browser settings)
                const url = URL.createObjectURL(blob);
                const a = document.createElement('a');
                a.href = url; a.download = suggestedName;
                document.body.appendChild(a); a.click(); document.body.removeChild(a);
                URL.revokeObjectURL(url);
            }
            return { success: true, successCount: files.length, errorCount: 0 };
        } catch (error) {
            console.error('saveFilesAsZip error:', error);
            return { success: false, error: error?.message || String(error) };
        }
    }

    /**
     * Get current configuration
     * @returns {Promise<Object>} Configuration object
     */
    async getConfig() {
        // Always refresh connection info before getting config
        this.refreshConnection();

        if (this.type === 'local') {
            return this.getLocalConfig();
        } else {
            return this.getRemoteConfig();
        }
    }

    /**
     * Update configuration item
     * @param {string} endpoint - Configuration item path
     * @param {any} value - Configuration value
     * @param {boolean} isDelete - Whether to delete configuration item
     * @returns {Promise<boolean>} Whether operation was successful
     */
    async updateSetting(endpoint, value, isDelete = false) {
        if (this.type === 'local') {
            return this.updateLocalSetting(endpoint, value, isDelete);
        } else {
            return this.updateRemoteSetting(endpoint, value, isDelete);
        }
    }

    /**
     * Get API key configuration
     * @param {string} keyType - Key type (gemini, codex, claude, openai)
     * @returns {Promise<Array>} Key array
     */
    async getApiKeys(keyType) {
        if (this.type === 'local') {
            return this.getLocalApiKeys(keyType);
        } else {
            return this.getRemoteApiKeys(keyType);
        }
    }

    /**
     * Update API key configuration
     * @param {string} keyType - Key type
     * @param {Array} keys - Key array
     * @returns {Promise<boolean>} Whether operation was successful
     */
    async updateApiKeys(keyType, keys) {
        if (this.type === 'local') {
            return this.updateLocalApiKeys(keyType, keys);
        } else {
            return this.updateRemoteApiKeys(keyType, keys);
        }
    }

    /**
     * Get authentication file list
     * @returns {Promise<Array>} File list
     */
    async getAuthFiles() {
        if (this.type === 'local') {
            return this.getLocalAuthFiles();
        } else {
            return this.getRemoteAuthFiles();
        }
    }

    /**
     * Upload authentication files
     * @param {File|Array<File>} files - Files to upload
     * @returns {Promise<Object>} Upload result
     */
    async uploadAuthFiles(files) {
        if (this.type === 'local') {
            return this.uploadLocalAuthFiles(files);
        } else {
            return this.uploadRemoteAuthFiles(files);
        }
    }

    /**
     * Delete authentication files
     * @param {string|Array<string>} filenames - Filenames to delete
     * @returns {Promise<Object>} Delete result
     */
    async deleteAuthFiles(filenames) {
        if (this.type === 'local') {
            return this.deleteLocalAuthFiles(filenames);
        } else {
            return this.deleteRemoteAuthFiles(filenames);
        }
    }

    /**
     * Download authentication files
     * @param {string|Array<string>} filenames - Filenames to download
     * @returns {Promise<Object>} Download result
     */
    async downloadAuthFiles(filenames) {
        if (this.type === 'local') {
            return this.downloadLocalAuthFiles(filenames);
        } else {
            return this.downloadRemoteAuthFiles(filenames);
        }
    }

    /**
     * Save Gemini Web tokens
     * @param {string} secure1psid - Secure-1PSID cookie value
     * @param {string} secure1psidts - Secure-1PSIDTS cookie value
     * @param {string} email - Email address (used as label)
     * @returns {Promise<Object>} Save result
     */
    async saveGeminiWebTokens(secure1psid, secure1psidts, email) {
        console.log('=== DEBUG: saveGeminiWebTokens ===');
        console.log('this.type:', this.type);
        if (this.type === 'local') {
            return this.saveLocalGeminiWebTokens(secure1psid, secure1psidts, email);
        } else {
            return this.saveRemoteGeminiWebTokens(secure1psid, secure1psidts, email);
        }
    }

    // ==================== Local Mode Implementation ====================

    /**
     * Get local configuration
     * @returns {Promise<Object>} Configuration object
     */
    async getLocalConfig() {
        try {
            if (window.__TAURI__?.core?.invoke) {
                const config = await window.__TAURI__.core.invoke('read_config_yaml');
                return config || {};
            }
            const configStr = localStorage.getItem('config');
            return configStr ? JSON.parse(configStr) : {};
        } catch (error) {
            console.error('Error reading local config:', error);
            return {};
        }
    }

    /**
     * Update local configuration item
     * @param {string} endpoint - Configuration item path
     * @param {any} value - Configuration value
     * @param {boolean} isDelete - Whether to delete configuration item
     * @returns {Promise<boolean>} Whether operation was successful
     */
    async updateLocalSetting(endpoint, value, isDelete = false) {
        try {
            if (window.__TAURI__?.core?.invoke) {
                const result = await window.__TAURI__.core.invoke('update_config_yaml', {
                    endpoint,
                    value,
                    is_delete: isDelete
                });
                return !!(result && result.success);
            }
            // Fallback to localStorage (testing only)
            const configStr = localStorage.getItem('config');
            const config = configStr ? JSON.parse(configStr) : {};
            const key = endpoint.split('/').pop();
            if (isDelete) { delete config[key]; } else { config[key] = value; }
            localStorage.setItem('config', JSON.stringify(config));
            return true;
        } catch (error) {
            console.error('Error updating local setting:', error);
            return false;
        }
    }

    /**
     * Get local API keys
     * @param {string} keyType - Key type
     * @returns {Promise<Array>} Key array
     */
    async getLocalApiKeys(keyType) {
        try {
            const config = await this.getLocalConfig();

            const keyMap = {
                'gemini': 'generative-language-api-key',
                'codex': 'codex-api-key',
                'claude': 'claude-api-key',
                'openai': 'openai-compatibility',
                'access-token': 'api-keys'
            };

            const key = keyMap[keyType];
            if (!key) {
                throw new Error(`Unknown key type: ${keyType}`);
            }

            const keys = config[key] || [];
            return Array.isArray(keys) ? keys : [];
        } catch (error) {
            console.error(`Error getting local ${keyType} keys:`, error);
            return [];
        }
    }

    /**
     * Update local API keys
     * @param {string} keyType - Key type
     * @param {Array} keys - Key array
     * @returns {Promise<boolean>} Whether operation was successful
     */
    async updateLocalApiKeys(keyType, keys) {
        try {
            const keyMap = {
                'gemini': 'generative-language-api-key',
                'codex': 'codex-api-key',
                'claude': 'claude-api-key',
                'openai': 'openai-compatibility',
                'access-token': 'api-keys'
            };

            const endpoint = keyMap[keyType];
            if (!endpoint) {
                throw new Error(`Unknown key type: ${keyType}`);
            }

            return await this.updateLocalSetting(endpoint, keys);
        } catch (error) {
            console.error(`Error updating local ${keyType} keys:`, error);
            return false;
        }
    }

    /**
     * Get local authentication file list
     * @returns {Promise<Array>} File list
     */
    async getLocalAuthFiles() {
        try {
            if (window.__TAURI__?.core?.invoke) {
                const files = await window.__TAURI__.core.invoke('read_local_auth_files');
                return files || [];
            }
            return [];
        } catch (error) {
            console.error('Error reading local auth files:', error);
            return [];
        }
    }

    /**
     * Upload local authentication files
     * @param {File|Array<File>} files - Files to upload
     * @returns {Promise<Object>} Upload result
     */
    async uploadLocalAuthFiles(files) {
        try {
            if (window.__TAURI__?.core?.invoke) {
                const fileArray = Array.isArray(files) ? files : [files];
                const fileData = [];
                for (const file of fileArray) {
                    const content = await this.readFileAsText(file);
                    fileData.push({ name: file.name, content });
                }
                const result = await window.__TAURI__.core.invoke('upload_local_auth_files', { files: fileData });
                return result;
            }
            return { success: false, error: 'Tauri environment required' };
        } catch (error) {
            console.error('Error uploading local auth files:', error);
            return {
                success: false,
                error: error.message
            };
        }
    }

    /**
     * Read file content as text
     * @param {File} file - File object
     * @returns {Promise<string>} File content
     */
    readFileAsText(file) {
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = (e) => resolve(e.target.result);
            reader.onerror = (e) => reject(new Error('File read failed'));
            reader.readAsText(file);
        });
    }

    /**
     * Delete local authentication files
     * @param {string|Array<string>} filenames - Filenames to delete
     * @returns {Promise<Object>} Delete result
     */
    async deleteLocalAuthFiles(filenames) {
        try {
            if (window.__TAURI__?.core?.invoke) {
                const filenameArray = Array.isArray(filenames) ? filenames : [filenames];
                const result = await window.__TAURI__.core.invoke('delete_local_auth_files', { filenames: filenameArray });
                return result;
            }
            return { success: false, error: 'Tauri environment required' };
        } catch (error) {
            console.error('Error deleting local auth files:', error);
            return {
                success: false,
                error: error.message
            };
        }
    }

    /**
     * Save local Gemini Web tokens
     * @param {string} secure1psid - Secure-1PSID cookie value
     * @param {string} secure1psidts - Secure-1PSIDTS cookie value
     * @param {string} email - Email address (used as label)
     * @returns {Promise<Object>} Save result
     */
    async saveLocalGeminiWebTokens(secure1psid, secure1psidts, email) {
        try {
            // Read configuration to get port
            const config = await this.getConfig();
            const port = config.port || 8317;
            const baseUrl = `http://127.0.0.1:${port}`;

            // In local mode, use the random password from localStorage (set during CLIProxyAPI startup)
            const password = localStorage.getItem('local-management-key') || '';

            if (!password) {
                throw new Error('Missing local management key. Please restart CLIProxyAPI.');
            }

            const apiUrl = baseUrl.endsWith('/')
                ? `${baseUrl}v0/management/gemini-web-token`
                : `${baseUrl}/v0/management/gemini-web-token`;

            console.log('Making request to apiUrl:', apiUrl);
            console.log('Using MANAGEMENT_KEY header with password');

            const response = await fetch(apiUrl, {
                method: 'POST',
                headers: {
                    'X-Management-Key': password,
                    'Content-Type': 'application/json'
                },
                body: JSON.stringify({
                    secure_1psid: secure1psid,
                    secure_1psidts: secure1psidts,
                    label: email
                })
            });

            if (response.ok) {
                const data = await response.json();
                return {
                    success: true,
                    file: data.file
                };
            } else {
                const errorData = await response.json().catch(() => ({}));
                return {
                    success: false,
                    error: errorData.error || `HTTP ${response.status}: ${response.statusText}`
                };
            }
        } catch (error) {
            console.error('Error saving local Gemini Web tokens:', error);
            return {
                success: false,
                error: error.message
            };
        }
    }

    /**
     * Download local authentication files
     * @param {string|Array<string>} filenames - Filenames to download
     * @returns {Promise<Object>} Download result
     */
    async downloadLocalAuthFiles(filenames) {
        try {
            if (window.__TAURI__?.core?.invoke) {
                const filenameArray = Array.isArray(filenames) ? filenames : [filenames];
                const result = await window.__TAURI__.core.invoke('download_local_auth_files', { filenames: filenameArray });

                if (result && result.success && result.files) {
                    // Prefer Tauri native directory picker + save
                    if (window.__TAURI__?.core?.invoke) {
                        try {
                            const saveRes = await window.__TAURI__.core.invoke('save_files_to_directory', { files: result.files });
                            return saveRes;
                        } catch (error) {
                            if (String(error).includes('User cancelled directory selection')) {
                                return { success: false, error: 'User cancelled directory selection' };
                            }
                            console.error('Tauri save_files_to_directory failed, falling back:', error);
                            // Fall through to browser methods
                        }
                    }

                    // If File System Access API is available, use directory picker
                    if (typeof window.showDirectoryPicker === 'function') {
                        try {
                            const directoryHandle = await window.showDirectoryPicker({
                                mode: 'readwrite'
                            });

                            if (!directoryHandle) {
                                return {
                                    success: false,
                                    error: 'User cancelled directory selection'
                                };
                            }

                            let successCount = 0;
                            for (const file of result.files) {
                                try {
                                    const fileHandle = await directoryHandle.getFileHandle(file.name, { create: true });
                                    const writable = await fileHandle.createWritable();
                                    await writable.write(file.content);
                                    await writable.close();
                                    successCount++;
                                } catch (error) {
                                    console.error(`Error downloading ${file.name}:`, error);
                                }
                            }

                            return {
                                success: successCount > 0,
                                successCount,
                                errorCount: result.files.length - successCount
                            };
                        } catch (error) {
                            if (error.name === 'AbortError') {
                                return {
                                    success: false,
                                    error: 'User cancelled directory selection'
                                };
                            } else {
                                console.error('Directory picker error, falling back to browser downloads:', error);
                                // Fall through to browser-download fallback below
                            }
                        }
                    }

                    // Fallback: bundle into a ZIP so user can choose save location
                    const zipName = `auth-files-${new Date().toISOString().replace(/[:T]/g, '-').split('.')[0]}.zip`;
                    return await this.saveFilesAsZip(result.files, zipName);
                } else {
                    return result;
                }
            }
            return { success: false, error: 'Tauri environment required' };
        } catch (error) {
            console.error('Error downloading local auth files:', error);
            return {
                success: false,
                error: error.message
            };
        }
    }

    // ==================== Remote Mode Implementation ====================

    /**
     * Get remote configuration
     * @returns {Promise<Object>} Configuration object
     */
    async getRemoteConfig() {
        try {
            console.log('=== DEBUG: getRemoteConfig ===');
            console.log('this.baseUrl:', this.baseUrl);
            console.log('this.password exists:', !!this.password);
            console.log('localStorage base-url:', localStorage.getItem('base-url'));

            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const configUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/config`
                : `${this.baseUrl}/v0/management/config`;

            console.log('Making request to configUrl:', configUrl);

            const response = await fetch(configUrl, {
                method: 'GET',
                headers: {
                    'Authorization': `Bearer ${this.password}`,
                    'Content-Type': 'application/json',
                    'Cache-Control': 'no-cache, no-store, must-revalidate',
                    'Pragma': 'no-cache',
                    'Expires': '0'
                }
            });

            if (response.ok) {
                const config = await response.json();
                localStorage.setItem('config', JSON.stringify(config));
                return config;
            } else {
                throw new Error(`Failed to get config: ${response.status}`);
            }
        } catch (error) {
            console.error('Error getting remote config:', error);
            throw error;
        }
    }

    /**
     * Update remote configuration item
     * @param {string} endpoint - Configuration item path
     * @param {any} value - Configuration value
     * @param {boolean} isDelete - Whether to delete configuration item
     * @returns {Promise<boolean>} Whether operation was successful
     */
    async updateRemoteSetting(endpoint, value, isDelete = false) {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const apiUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/${endpoint}`
                : `${this.baseUrl}/v0/management/${endpoint}`;

            let response;
            if (isDelete) {
                response = await fetch(apiUrl, {
                    method: 'DELETE',
                    headers: {
                        'Authorization': `Bearer ${this.password}`
                    }
                });
            } else {
                const body = Array.isArray(value) ? JSON.stringify(value) : JSON.stringify({ value: value });
                response = await fetch(apiUrl, {
                    method: 'PUT',
                    headers: {
                        'Authorization': `Bearer ${this.password}`,
                        'Content-Type': 'application/json'
                    },
                    body: body
                });
            }

            return response.ok;
        } catch (error) {
            console.error(`Error updating remote setting ${endpoint}:`, error);
            return false;
        }
    }

    /**
     * Get remote API keys
     * @param {string} keyType - Key type
     * @returns {Promise<Array>} Key array
     */
    async getRemoteApiKeys(keyType) {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const keyMap = {
                'gemini': 'generative-language-api-key',
                'codex': 'codex-api-key',
                'claude': 'claude-api-key',
                'openai': 'openai-compatibility',
                'access-token': 'api-keys'
            };

            const endpoint = keyMap[keyType];
            if (!endpoint) {
                throw new Error(`Unknown key type: ${keyType}`);
            }

            const apiUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/${endpoint}`
                : `${this.baseUrl}/v0/management/${endpoint}`;

            const response = await fetch(apiUrl, {
                method: 'GET',
                headers: {
                    'Authorization': `Bearer ${this.password}`,
                    'Content-Type': 'application/json'
                }
            });

            if (response.ok) {
                const data = await response.json();
                const keys = data[endpoint] || [];
                return Array.isArray(keys) ? keys : [];
            } else {
                throw new Error(`Failed to load ${keyType} keys: ${response.status}`);
            }
        } catch (error) {
            console.error(`Error getting remote ${keyType} keys:`, error);
            return [];
        }
    }

    /**
     * Update remote API keys
     * @param {string} keyType - Key type
     * @param {Array} keys - Key array
     * @returns {Promise<boolean>} Whether operation was successful
     */
    async updateRemoteApiKeys(keyType, keys) {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const keyMap = {
                'gemini': 'generative-language-api-key',
                'codex': 'codex-api-key',
                'claude': 'claude-api-key',
                'openai': 'openai-compatibility',
                'access-token': 'api-keys'
            };

            const endpoint = keyMap[keyType];
            if (!endpoint) {
                throw new Error(`Unknown key type: ${keyType}`);
            }

            const apiUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/${endpoint}`
                : `${this.baseUrl}/v0/management/${endpoint}`;

            const response = await fetch(apiUrl, {
                method: 'PUT',
                headers: {
                    'Authorization': `Bearer ${this.password}`,
                    'Content-Type': 'application/json'
                },
                body: JSON.stringify(keys)
            });

            return response.ok;
        } catch (error) {
            console.error(`Error updating remote ${keyType} keys:`, error);
            return false;
        }
    }

    /**
     * Get remote authentication file list
     * @returns {Promise<Array>} File list
     */
    async getRemoteAuthFiles() {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const apiUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/auth-files`
                : `${this.baseUrl}/v0/management/auth-files`;

            const response = await fetch(apiUrl, {
                method: 'GET',
                headers: {
                    'Authorization': `Bearer ${this.password}`,
                    'Content-Type': 'application/json'
                }
            });

            if (response.ok) {
                const data = await response.json();
                return data.files || [];
            } else {
                throw new Error(`Failed to load auth files: ${response.status}`);
            }
        } catch (error) {
            console.error('Error getting remote auth files:', error);
            return [];
        }
    }

    /**
     * Upload remote authentication files
     * @param {File|Array<File>} files - Files to upload
     * @returns {Promise<Object>} Upload result
     */
    async uploadRemoteAuthFiles(files) {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const apiUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/auth-files`
                : `${this.baseUrl}/v0/management/auth-files`;

            const fileArray = Array.isArray(files) ? files : [files];
            let successCount = 0;
            let errorCount = 0;
            const errors = [];

            // Upload files sequentially to avoid server overload
            for (const file of fileArray) {
                try {
                    await this.uploadSingleFile(file, apiUrl);
                    successCount++;
                } catch (error) {
                    console.error(`Error uploading ${file.name}:`, error);
                    errorCount++;
                    errors.push(`${file.name}: ${error.message}`);
                }
            }

            return {
                success: successCount > 0,
                successCount,
                errorCount,
                errors: errors.length > 0 ? errors : undefined
            };
        } catch (error) {
            console.error('Error uploading remote auth files:', error);
            return {
                success: false,
                error: error.message
            };
        }
    }

    /**
     * Upload single file
     * @param {File} file - File to upload
     * @param {string} apiUrl - API URL
     * @returns {Promise<void>}
     */
    async uploadSingleFile(file, apiUrl) {
        const formData = new FormData();
        formData.append('file', file);

        const response = await fetch(apiUrl, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${this.password}`
            },
            body: formData
        });

        if (!response.ok) {
            let errorMessage = 'Upload failed';

            if (response.status === 401) {
                errorMessage = 'Authentication failed';
            } else if (response.status === 403) {
                errorMessage = 'Access denied';
            } else if (response.status === 413) {
                errorMessage = 'File too large';
            } else if (response.status >= 500) {
                errorMessage = 'Server error';
            }

            throw new Error(errorMessage);
        }
    }

    /**
     * Delete remote authentication files
     * @param {string|Array<string>} filenames - Filenames to delete
     * @returns {Promise<Object>} Delete result
     */
    async deleteRemoteAuthFiles(filenames) {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const filenameArray = Array.isArray(filenames) ? filenames : [filenames];
            let successCount = 0;
            let errorCount = 0;

            for (const filename of filenameArray) {
                try {
                    const apiUrl = this.baseUrl.endsWith('/')
                        ? `${this.baseUrl}v0/management/auth-files?name=${encodeURIComponent(filename)}`
                        : `${this.baseUrl}/v0/management/auth-files?name=${encodeURIComponent(filename)}`;

                    const response = await fetch(apiUrl, {
                        method: 'DELETE',
                        headers: {
                            'Authorization': `Bearer ${this.password}`
                        }
                    });

                    if (response.ok) {
                        successCount++;
                    } else {
                        errorCount++;
                    }
                } catch (error) {
                    console.error(`Error deleting ${filename}:`, error);
                    errorCount++;
                }
            }

            return {
                success: successCount > 0,
                successCount,
                errorCount
            };
        } catch (error) {
            console.error('Error deleting remote auth files:', error);
            return {
                success: false,
                error: error.message
            };
        }
    }

    /**
     * Download remote authentication files
     * @param {string|Array<string>} filenames - Filenames to download
     * @returns {Promise<Object>} Download result
     */
    async downloadRemoteAuthFiles(filenames) {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const filenameArray = Array.isArray(filenames) ? filenames : [filenames];
            let successCount = 0;
            let errorCount = 0;
            let usedDirectoryPicker = false;

            // Prefer Tauri native directory picker + save (best UX across browsers)
            if (window.__TAURI__?.core?.invoke) {
                try {
                    // Fetch all files as text, then ask Tauri to save to a chosen directory
                    const files = [];
                    for (const filename of filenameArray) {
                        const apiUrl = this.baseUrl.endsWith('/')
                            ? `${this.baseUrl}v0/management/auth-files/download?name=${encodeURIComponent(filename)}`
                            : `${this.baseUrl}/v0/management/auth-files/download?name=${encodeURIComponent(filename)}`;
                        const response = await fetch(apiUrl, {
                            method: 'GET',
                            headers: { 'Authorization': `Bearer ${this.password}` }
                        });
                        if (!response.ok) {
                            throw new Error(`HTTP ${response.status}: ${response.statusText}`);
                        }
                        const content = await response.text();
                        files.push({ name: filename, content });
                    }
                    const saveRes = await window.__TAURI__.core.invoke('save_files_to_directory', { files });
                    return saveRes;
                } catch (error) {
                    if (String(error).includes('User cancelled directory selection')) {
                        return { success: false, error: 'User cancelled directory selection' };
                    }
                    console.error('Tauri save_files_to_directory fallback failed; trying browser options:', error);
                    // Fall through to browser fallbacks below
                }
            }

            // Prefer File System Access API when available
            if (typeof window.showDirectoryPicker === 'function') {
                try {
                    const directoryHandle = await window.showDirectoryPicker({
                        mode: 'readwrite'
                    });

                    if (!directoryHandle) {
                        return {
                            success: false,
                            error: 'User cancelled directory selection'
                        };
                    }

                    usedDirectoryPicker = true;

                    // Download each file into selected directory
                    for (const filename of filenameArray) {
                        try {
                            await this.downloadSingleFile(filename, directoryHandle);
                            successCount++;
                        } catch (error) {
                            console.error(`Error downloading ${filename}:`, error);
                            errorCount++;
                        }
                    }

                    return {
                        success: successCount > 0,
                        successCount,
                        errorCount
                    };
                } catch (error) {
                    if (error.name === 'AbortError') {
                        return {
                            success: false,
                            error: 'User cancelled directory selection'
                        };
                    }
                    console.error('Directory picker unavailable or failed; falling back:', error);
                    // Fall through to browser-download fallback below
                }
            }

            // Fallback: fetch all files and save as a ZIP via Save As dialog if available
            const files = [];
            for (const filename of filenameArray) {
                try {
                    const apiUrl = this.baseUrl.endsWith('/')
                        ? `${this.baseUrl}v0/management/auth-files/download?name=${encodeURIComponent(filename)}`
                        : `${this.baseUrl}/v0/management/auth-files/download?name=${encodeURIComponent(filename)}`;
                    const response = await fetch(apiUrl, {
                        method: 'GET',
                        headers: { 'Authorization': `Bearer ${this.password}` }
                    });
                    if (!response.ok) { throw new Error(`HTTP ${response.status}: ${response.statusText}`); }
                    const content = await response.text();
                    files.push({ name: filename, content });
                } catch (e) {
                    console.error(`Error downloading ${filename}:`, e);
                    errorCount++;
                }
            }
            if (files.length > 0) {
                const zipName = `auth-files-${new Date().toISOString().replace(/[:T]/g, '-').split('.')[0]}.zip`;
                const res = await this.saveFilesAsZip(files, zipName);
                if (res.success) {
                    return { success: true, successCount: files.length, errorCount };
                }
                return { success: false, error: res.error || 'Failed to save ZIP', errorCount };
            }
            return { success: false, error: 'No file downloaded', errorCount };
        } catch (error) {
            if (error && error.name === 'AbortError') {
                return {
                    success: false,
                    error: 'User cancelled directory selection'
                };
            }
            console.error('Error downloading remote auth files:', error);
            return {
                success: false,
                error: error?.message || String(error)
            };
        }
    }

    /**
     * Save remote Gemini Web tokens
     * @param {string} secure1psid - Secure-1PSID cookie value
     * @param {string} secure1psidts - Secure-1PSIDTS cookie value
     * @param {string} email - Email address (used as label)
     * @returns {Promise<Object>} Save result
     */
    async saveRemoteGeminiWebTokens(secure1psid, secure1psidts, email) {
        try {
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const apiUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/gemini-web-token`
                : `${this.baseUrl}/v0/management/gemini-web-token`;

            const response = await fetch(apiUrl, {
                method: 'POST',
                headers: {
                    'Authorization': `Bearer ${this.password}`,
                    'Content-Type': 'application/json'
                },
                body: JSON.stringify({
                    secure_1psid: secure1psid,
                    secure_1psidts: secure1psidts,
                    label: email
                })
            });

            if (response.ok) {
                const data = await response.json();
                return {
                    success: true,
                    file: data.file
                };
            } else {
                const errorData = await response.json().catch(() => ({}));
                return {
                    success: false,
                    error: errorData.error || `HTTP ${response.status}: ${response.statusText}`
                };
            }
        } catch (error) {
            console.error('Error saving remote Gemini Web tokens:', error);
            return {
                success: false,
                error: error.message
            };
        }
    }

    /**
     * Download single file to directory
     * @param {string} filename - Filename
     * @param {FileSystemDirectoryHandle} directoryHandle - Directory handle
     * @returns {Promise<void>}
     */
    async downloadSingleFile(filename, directoryHandle) {
        const apiUrl = this.baseUrl.endsWith('/')
            ? `${this.baseUrl}v0/management/auth-files/download?name=${encodeURIComponent(filename)}`
            : `${this.baseUrl}/v0/management/auth-files/download?name=${encodeURIComponent(filename)}`;

        const response = await fetch(apiUrl, {
            method: 'GET',
            headers: {
                'Authorization': `Bearer ${this.password}`
            }
        });

        if (!response.ok) {
            throw new Error(`HTTP ${response.status}: ${response.statusText}`);
        }

        // Get file content
        const fileContent = await response.blob();

        // Create file in directory
        const fileHandle = await directoryHandle.getFileHandle(filename, { create: true });
        const writable = await fileHandle.createWritable();
        await writable.write(fileContent);
        await writable.close();
    }

    /**
     * Start keep-alive mechanism for Local mode (backend implementation)
     */
    async startKeepAlive() {
        if (this.type !== 'local' || this.keepAliveEnabled) {
            return;
        }

        try {
            // Get current configuration to determine port
            const config = await this.getLocalConfig();
            const port = config.port || 8317;

            if (window.__TAURI__?.core?.invoke) {
                console.log('Starting keep-alive mechanism for Local mode via backend');
                const result = await window.__TAURI__.core.invoke('start_keep_alive', {
                    port: port
                });

                if (result && result.success) {
                    this.keepAliveEnabled = true;
                    console.log('Keep-alive mechanism started successfully');
                } else {
                    console.error('Failed to start keep-alive mechanism');
                }
            } else {
                console.warn('Tauri environment not available for keep-alive');
            }
        } catch (error) {
            console.error('Error starting keep-alive mechanism:', error);
        }
    }

    /**
     * Stop keep-alive mechanism (backend implementation)
     */
    async stopKeepAlive() {
        if (!this.keepAliveEnabled) {
            return;
        }

        try {
            if (window.__TAURI__?.core?.invoke) {
                console.log('Stopping keep-alive mechanism via backend');
                const result = await window.__TAURI__.core.invoke('stop_keep_alive');

                if (result && result.success) {
                    this.keepAliveEnabled = false;
                    console.log('Keep-alive mechanism stopped successfully');
                } else {
                    console.error('Failed to stop keep-alive mechanism');
                }
            } else {
                console.warn('Tauri environment not available for keep-alive');
            }
        } catch (error) {
            console.error('Error stopping keep-alive mechanism:', error);
        }
    }

    /**
     * Refresh connection information
     */
    refreshConnection() {
        const oldBaseUrl = this.baseUrl;
        const oldType = this.type;

        this.type = localStorage.getItem('type') || 'local';
        this.baseUrl = localStorage.getItem('base-url');
        this.password = localStorage.getItem('password');

        console.log('=== DEBUG: refreshConnection ===');
        console.log('Old baseUrl:', oldBaseUrl);
        console.log('New baseUrl:', this.baseUrl);
        console.log('Old type:', oldType);
        console.log('New type:', this.type);

        // Handle keep-alive based on type change
        if (oldType !== this.type) {
            if (oldType === 'local') {
                this.stopKeepAlive().catch(error => {
                    console.error('Error stopping keep-alive during type change:', error);
                });
            }
            if (this.type === 'local') {
                this.startKeepAlive().catch(error => {
                    console.error('Error starting keep-alive during type change:', error);
                });
            }
        }

        // Clear any cached config to ensure fresh connection
        localStorage.removeItem('config');
    }
}

// Create global instance
window.configManager = new ConfigManager();
