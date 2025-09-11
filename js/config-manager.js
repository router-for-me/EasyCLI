/**
 * Configuration Manager Abstraction Layer
 * Unified operation interface for Local and Remote modes
 */
class ConfigManager {
    constructor() {
        this.type = localStorage.getItem('type') || 'local';
        this.baseUrl = localStorage.getItem('base-url');
        this.password = localStorage.getItem('password');
    }

    /**
     * Get current configuration
     * @returns {Promise<Object>} Configuration object
     */
    async getConfig() {
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
                const result = await window.__TAURI__.core.invoke('upload_local_auth_files', fileData);
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
                const result = await window.__TAURI__.core.invoke('delete_local_auth_files', filenameArray);
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
     * Download local authentication files
     * @param {string|Array<string>} filenames - Filenames to download
     * @returns {Promise<Object>} Download result
     */
    async downloadLocalAuthFiles(filenames) {
        try {
            if (window.__TAURI__?.core?.invoke) {
                const filenameArray = Array.isArray(filenames) ? filenames : [filenames];
                const result = await window.__TAURI__.core.invoke('download_local_auth_files', filenameArray);

                if (result && result.success && result.files) {
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

                    // Fallback: trigger browser downloads for each file
                    let successCount = 0;
                    for (const file of result.files) {
                        try {
                            const blob = new Blob([file.content]);
                            const url = URL.createObjectURL(blob);
                            const a = document.createElement('a');
                            a.href = url;
                            a.download = file.name;
                            document.body.appendChild(a);
                            a.click();
                            document.body.removeChild(a);
                            URL.revokeObjectURL(url);
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
            if (!this.baseUrl || !this.password) {
                throw new Error('Missing connection information');
            }

            const configUrl = this.baseUrl.endsWith('/')
                ? `${this.baseUrl}v0/management/config`
                : `${this.baseUrl}/v0/management/config`;

            const response = await fetch(configUrl, {
                method: 'GET',
                headers: {
                    'Authorization': `Bearer ${this.password}`,
                    'Content-Type': 'application/json'
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

            // Fallback: download each file via browser (to default downloads folder)
            for (const filename of filenameArray) {
                try {
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

                    const blob = await response.blob();
                    const url = URL.createObjectURL(blob);
                    const a = document.createElement('a');
                    a.href = url;
                    a.download = filename;
                    document.body.appendChild(a);
                    a.click();
                    document.body.removeChild(a);
                    URL.revokeObjectURL(url);
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
     * Refresh connection information
     */
    refreshConnection() {
        this.type = localStorage.getItem('type') || 'local';
        this.baseUrl = localStorage.getItem('base-url');
        this.password = localStorage.getItem('password');
    }
}

// Create global instance
window.configManager = new ConfigManager();
