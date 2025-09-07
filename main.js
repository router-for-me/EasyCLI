const { app, BrowserWindow, Menu, ipcMain } = require('electron');
const path = require('path');
const fs = require('fs');
const https = require('https');
const tar = require('tar');
const os = require('os');
const AdmZip = require('adm-zip');
const { spawn } = require('child_process');
const yaml = require('js-yaml');

// Parse path, support ~ symbol (user home directory)
function resolvePath(inputPath, basePath = null) {
    if (!inputPath || inputPath === '') {
        return null;
    }

    // Handle ~ symbol
    if (inputPath.startsWith('~')) {
        const homeDir = os.homedir();
        if (inputPath === '~') {
            return homeDir;
        } else if (inputPath.startsWith('~/')) {
            return path.join(homeDir, inputPath.slice(2));
        } else {
            // Handle ~username case (if needed)
            return path.join(homeDir, inputPath.slice(1));
        }
    }

    // Handle absolute path
    if (path.isAbsolute(inputPath)) {
        return inputPath;
    }

    // Handle relative path
    if (basePath) {
        return path.join(basePath, inputPath);
    }

    return inputPath;
}

let mainWindow;
let cliProxyApiProcess = null;
let processMonitorInterval = null;

// Function to get latest version information
async function getLatestReleaseInfo() {
    return new Promise((resolve, reject) => {
        const options = {
            hostname: 'api.github.com',
            path: '/repos/luispater/CLIProxyAPI/releases/latest',
            method: 'GET',
            headers: {
                'User-Agent': 'CLIProxyAPI-GUI',
                'Accept': 'application/vnd.github.v3+json'
            }
        };

        https.get(options, (response) => {
            let data = '';

            response.on('data', (chunk) => {
                data += chunk;
            });

            response.on('end', () => {
                try {
                    const releaseInfo = JSON.parse(data);
                    resolve(releaseInfo);
                } catch (error) {
                    reject(new Error('Failed to parse version information: ' + error.message));
                }
            });
        }).on('error', (error) => {
            reject(new Error('Failed to get version information: ' + error.message));
        });
    });
}

// Check if local executable file exists
function checkLocalExecutable() {
    const downloadDir = path.join(os.homedir(), 'cliproxyapi');
    const versionFile = path.join(downloadDir, 'version.txt');

    // Check if version file exists
    if (!fs.existsSync(versionFile)) {
        return { exists: false };
    }

    // Read current version
    const currentVersion = fs.readFileSync(versionFile, 'utf8').trim();
    const extractPath = path.join(downloadDir, currentVersion);

    // Check if version directory exists
    if (!fs.existsSync(extractPath)) {
        return { exists: false };
    }

    // Find executable file
    const platform = os.platform();
    let executableName = 'cli-proxy-api';
    if (platform === 'win32') {
        executableName += '.exe';
    }

    const executablePath = path.join(extractPath, executableName);
    if (!fs.existsSync(executablePath)) {
        return { exists: false };
    }

    return {
        exists: true,
        version: currentVersion,
        path: extractPath,
        executablePath: executablePath
    };
}

// Compare version numbers
function compareVersions(version1, version2) {
    const v1parts = version1.split('.').map(Number);
    const v2parts = version2.split('.').map(Number);

    for (let i = 0; i < Math.max(v1parts.length, v2parts.length); i++) {
        const v1part = v1parts[i] || 0;
        const v2part = v2parts[i] || 0;

        if (v1part > v2part) return 1;
        if (v1part < v2part) return -1;
    }

    return 0;
}

// Check and copy configuration file
function ensureConfigFile(versionPath) {
    const downloadDir = path.join(os.homedir(), 'cliproxyapi');
    const configPath = path.join(downloadDir, 'config.yaml');
    const exampleConfigPath = path.join(versionPath, 'config.example.yaml');

    try {
        // Check if config.yaml exists
        if (fs.existsSync(configPath)) {
            console.log('config.yaml already exists, skip copying');
            return { success: true, message: 'config.yaml already exists' };
        }

        // Check if config.example.yaml exists
        if (!fs.existsSync(exampleConfigPath)) {
            console.log('config.example.yaml does not exist, skip copying');
            return { success: true, message: 'config.example.yaml does not exist' };
        }

        // Copy config.example.yaml to config.yaml
        fs.copyFileSync(exampleConfigPath, configPath);
        console.log('Copied config.example.yaml to config.yaml');

        return { success: true, message: 'config.yaml created' };
    } catch (error) {
        console.error('Failed to copy configuration file:', error);
        return { success: false, error: error.message };
    }
}

// Start CLIProxyAPI process
function startCLIProxyAPI(versionPath) {
    const downloadDir = path.join(os.homedir(), 'cliproxyapi');
    const configPath = path.join(downloadDir, 'config.yaml');

    // Find executable file
    const platform = os.platform();
    let executableName = 'cli-proxy-api';
    if (platform === 'win32') {
        executableName += '.exe';
    }

    const executablePath = path.join(versionPath, executableName);

    if (!fs.existsSync(executablePath)) {
        console.error('CLIProxyAPI executable file does not exist:', executablePath);
        return { success: false, error: 'Executable file does not exist' };
    }

    if (!fs.existsSync(configPath)) {
        console.error('Configuration file does not exist:', configPath);
        return { success: false, error: 'Configuration file does not exist' };
    }

    try {
        // Start CLIProxyAPI process
        cliProxyApiProcess = spawn(executablePath, ['-config', configPath], {
            detached: false,
            stdio: ['ignore', 'pipe', 'pipe']
        });

        console.log(`CLIProxyAPI process started, PID: ${cliProxyApiProcess.pid}`);

        // Listen for process output
        cliProxyApiProcess.stdout.on('data', (data) => {
            console.log(`CLIProxyAPI stdout: ${data}`);
        });

        cliProxyApiProcess.stderr.on('data', (data) => {
            console.log(`CLIProxyAPI stderr: ${data}`);
        });

        // Listen for process exit
        cliProxyApiProcess.on('exit', (code, signal) => {
            console.log(`CLIProxyAPI process exited, code: ${code}, signal: ${signal}`);

            // Stop process monitoring
            stopProcessMonitor();

            // If process exits abnormally (not normal exit), notify frontend
            if (code !== 0 && code !== null) {
                console.error(`CLIProxyAPI process exited abnormally, exit code: ${code}`);
                if (mainWindow) {
                    mainWindow.webContents.send('process-exit-error', {
                        error: 'CLIProxyAPI process exited abnormally',
                        code: code,
                        signal: signal
                    });
                }
            }

            cliProxyApiProcess = null;
        });

        cliProxyApiProcess.on('error', (error) => {
            console.error('CLIProxyAPI process error:', error);

            // Stop process monitoring
            stopProcessMonitor();

            // Notify frontend of process start error
            if (mainWindow) {
                mainWindow.webContents.send('process-start-error', {
                    error: 'CLIProxyAPI process start error',
                    reason: error.message
                });
            }

            cliProxyApiProcess = null;
        });

        return { success: true, pid: cliProxyApiProcess.pid };
    } catch (error) {
        console.error('Failed to start CLIProxyAPI process:', error);
        return { success: false, error: error.message };
    }
}

// Stop CLIProxyAPI process
function stopCLIProxyAPI() {
    // Stop process monitoring
    stopProcessMonitor();

    if (cliProxyApiProcess) {
        try {
            console.log(`Stopping CLIProxyAPI process, PID: ${cliProxyApiProcess.pid}`);
            cliProxyApiProcess.kill('SIGTERM');

            // Wait for process to exit gracefully
            setTimeout(() => {
                if (cliProxyApiProcess && !cliProxyApiProcess.killed) {
                    console.log('Force killing CLIProxyAPI process');
                    cliProxyApiProcess.kill('SIGKILL');
                }
            }, 5000);

            cliProxyApiProcess = null;
            return { success: true };
        } catch (error) {
            console.error('Failed to stop CLIProxyAPI process:', error);
            return { success: false, error: error.message };
        }
    }
    return { success: true, message: 'No running process' };
}

// Start monitoring CLIProxyAPI process
function startProcessMonitor() {
    if (processMonitorInterval) {
        clearInterval(processMonitorInterval);
    }

    processMonitorInterval = setInterval(() => {
        if (cliProxyApiProcess) {
            // Check if process is still running
            try {
                // Send signal 0 to check if process exists
                process.kill(cliProxyApiProcess.pid, 0);
            } catch (error) {
                // Process no longer exists
                console.log('CLIProxyAPI process has closed, stopping monitoring');
                cliProxyApiProcess = null;
                clearInterval(processMonitorInterval);
                processMonitorInterval = null;

                // Notify frontend that process has closed
                if (mainWindow) {
                    mainWindow.webContents.send('process-closed', {
                        message: 'CLIProxyAPI process has closed'
                    });
                }
            }
        } else {
            // Process object doesn't exist, stop monitoring
            clearInterval(processMonitorInterval);
            processMonitorInterval = null;
        }
    }, 2000); // Check every 2 seconds
}

// Stop monitoring CLIProxyAPI process
function stopProcessMonitor() {
    if (processMonitorInterval) {
        clearInterval(processMonitorInterval);
        processMonitorInterval = null;
    }
}

// Restart CLIProxyAPI process
function restartCLIProxyAPI() {
    console.log('Starting CLIProxyAPI process restart...');

    // Stop current process
    stopCLIProxyAPI();

    // Wait for process to completely stop
    setTimeout(() => {
        // Restart process
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const versionFile = path.join(downloadDir, 'version.txt');

        if (fs.existsSync(versionFile)) {
            const currentVersion = fs.readFileSync(versionFile, 'utf8').trim();
            const versionPath = path.join(downloadDir, currentVersion);

            const startResult = startCLIProxyAPI(versionPath);
            if (startResult.success) {
                console.log(`CLIProxyAPI process restarted successfully, PID: ${startResult.pid}`);

                // Start monitoring process
                startProcessMonitor();

                // Notify frontend of successful restart
                if (mainWindow) {
                    mainWindow.webContents.send('cliproxyapi-restarted', {
                        success: true,
                        message: 'CLIProxyAPI process restarted successfully'
                    });
                }
            } else {
                console.error('CLIProxyAPI process restart failed:', startResult.error);

                // Notify frontend of restart failure
                if (mainWindow) {
                    mainWindow.webContents.send('cliproxyapi-restart-failed', {
                        success: false,
                        error: startResult.error
                    });
                }
            }
        } else {
            console.error('Version file does not exist, cannot restart CLIProxyAPI');

            // Notify frontend of restart failure
            if (mainWindow) {
                mainWindow.webContents.send('cliproxyapi-restart-failed', {
                    success: false,
                    error: 'Version file does not exist'
                });
            }
        }
    }, 1000); // Wait 1 second to ensure process is completely stopped
}

// Check secret-key in config.yaml
function checkSecretKey() {
    const downloadDir = path.join(os.homedir(), 'cliproxyapi');
    const configPath = path.join(downloadDir, 'config.yaml');

    try {
        if (!fs.existsSync(configPath)) {
            return { needsPassword: true, reason: 'Configuration file does not exist' };
        }

        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent);

        // Check if remote-management.secret-key exists and is not empty
        if (!config || !config['remote-management'] || !config['remote-management']['secret-key'] ||
            config['remote-management']['secret-key'].trim() === '') {
            return { needsPassword: true, reason: 'secret-key is empty' };
        }

        return { needsPassword: false, secretKey: config['remote-management']['secret-key'] };
    } catch (error) {
        console.error('Failed to check secret-key:', error);
        return { needsPassword: true, reason: 'Configuration file parsing failed' };
    }
}

// Update secret-key in config.yaml
function updateSecretKey(newSecretKey) {
    const downloadDir = path.join(os.homedir(), 'cliproxyapi');
    const configPath = path.join(downloadDir, 'config.yaml');

    try {
        if (!fs.existsSync(configPath)) {
            return { success: false, error: 'Configuration file does not exist' };
        }

        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent);

        // Ensure remote-management structure exists
        if (!config) {
            config = {};
        }
        if (!config['remote-management']) {
            config['remote-management'] = {};
        }

        // Update secret-key
        config['remote-management']['secret-key'] = newSecretKey;

        // Write back to file
        const updatedContent = yaml.dump(config, {
            indent: 2,
            lineWidth: -1,
            noRefs: true
        });

        fs.writeFileSync(configPath, updatedContent, 'utf8');
        console.log('Updated secret-key in config.yaml');

        return { success: true };
    } catch (error) {
        console.error('Failed to update secret-key:', error);
        return { success: false, error: error.message };
    }
}

// Check version and decide whether update is needed
async function checkVersionAndUpdate() {
    const downloadDir = path.join(os.homedir(), 'cliproxyapi');

    try {
        // Ensure download directory exists
        if (!fs.existsSync(downloadDir)) {
            fs.mkdirSync(downloadDir, { recursive: true });
        }

        // Check if local executable file exists
        const localInfo = checkLocalExecutable();

        // Get latest version information
        console.log('Getting latest version information...');
        const releaseInfo = await getLatestReleaseInfo();
        const latestVersion = releaseInfo.tag_name.replace(/^v/, ''); // Remove v prefix from version number
        console.log(`Latest version: ${latestVersion}`);

        // If no local files, download directly
        if (!localInfo.exists) {
            console.log('CLIProxyAPI not found locally, starting download...');
            return await downloadAndExtractCLIProxyAPI(latestVersion, releaseInfo);
        }

        // Compare versions
        const versionComparison = compareVersions(localInfo.version, latestVersion);

        if (versionComparison >= 0) {
            // Version is latest or newer
            console.log(`Local version ${localInfo.version} is already the latest version`);

            // Check configuration file
            const configResult = ensureConfigFile(localInfo.path);
            if (!configResult.success) {
                console.warn('Configuration file processing failed:', configResult.error);
            }

            return {
                success: true,
                path: localInfo.path,
                version: localInfo.version,
                needsUpdate: false,
                isLatest: true
            };
        } else {
            // Update needed
            console.log(`Local version ${localInfo.version} needs to be updated to ${latestVersion}`);
            return {
                success: true,
                path: localInfo.path,
                version: localInfo.version,
                latestVersion: latestVersion,
                needsUpdate: true,
                isLatest: false
            };
        }
    } catch (error) {
        console.error('Version check failed:', error);
        return { success: false, error: error.message };
    }
}

// Function to download and extract CLIProxyAPI
async function downloadAndExtractCLIProxyAPI(version, releaseInfo) {
    const downloadDir = path.join(os.homedir(), 'cliproxyapi');
    const extractPath = path.join(downloadDir, version);

    try {
        // Find download link suitable for current platform
        const platform = os.platform();
        const arch = os.arch();
        let downloadUrl = null;
        let fileName = null;

        // Select appropriate download file based on platform and architecture
        if (platform === 'darwin') {
            if (arch === 'arm64') {
                fileName = `CLIProxyAPI_${version}_darwin_arm64.tar.gz`;
            } else if (arch === 'x64') {
                fileName = `CLIProxyAPI_${version}_darwin_amd64.tar.gz`;
            }
        } else if (platform === 'linux') {
            if (arch === 'x64') {
                fileName = `CLIProxyAPI_${version}_linux_amd64.tar.gz`;
            } else if (arch === 'arm64') {
                fileName = `CLIProxyAPI_${version}_linux_arm64.tar.gz`;
            }
        } else if (platform === 'win32') {
            if (arch === 'x64') {
                fileName = `CLIProxyAPI_${version}_windows_amd64.zip`;
            } else if (arch === 'arm64') {
                fileName = `CLIProxyAPI_${version}_windows_arm64.zip`;
            }
        }

        if (!fileName) {
            throw new Error(`Unsupported operating system: ${platform} ${arch}`);
        }

        // Find corresponding file in release assets
        const asset = releaseInfo.assets.find(asset => asset.name === fileName);
        if (!asset) {
            throw new Error(`No suitable download file found: ${fileName}`);
        }

        downloadUrl = asset.browser_download_url;
        const downloadPath = path.join(downloadDir, fileName);

        console.log(`Found download link: ${downloadUrl}`);
        console.log(`Starting download of CLIProxyAPI ${version}...`);

        // Download file with progress callback
        await downloadFile(downloadUrl, downloadPath, (progressData) => {
            // Send progress update to frontend
            if (mainWindow) {
                mainWindow.webContents.send('download-progress', progressData);
            }
        });
        console.log('Download completed, starting extraction...');

        // Choose extraction method based on file extension
        if (fileName.endsWith('.zip')) {
            await extractZip(downloadPath, extractPath);
        } else {
            await extractTarGz(downloadPath, extractPath);
        }
        console.log('Extraction completed');

        // Save version information
        const versionFile = path.join(downloadDir, 'version.txt');
        const oldVersion = fs.existsSync(versionFile) ? fs.readFileSync(versionFile, 'utf8').trim() : null;
        fs.writeFileSync(versionFile, version);
        console.log(`Version information saved: ${version}`);

        // Clean up old version directory (if exists and not current version)
        if (oldVersion && oldVersion !== version) {
            const oldVersionPath = path.join(downloadDir, oldVersion);
            if (fs.existsSync(oldVersionPath)) {
                try {
                    fs.rmSync(oldVersionPath, { recursive: true, force: true });
                    console.log(`Cleaned up old version directory: ${oldVersion}`);
                } catch (error) {
                    console.warn(`Failed to clean up old version directory: ${error.message}`);
                }
            }
        }

        // Delete downloaded tar.gz file
        fs.unlinkSync(downloadPath);
        console.log('Temporary file cleanup completed');

        // Check and copy configuration file
        const configResult = ensureConfigFile(extractPath);
        if (!configResult.success) {
            console.warn('Configuration file processing failed:', configResult.error);
        }

        return { success: true, path: extractPath, version: version };
    } catch (error) {
        console.error('Download or extraction failed:', error);
        return { success: false, error: error.message };
    }
}

// Function to download files, supports following redirects and progress updates
function downloadFile(url, filePath, progressCallback) {
    return new Promise((resolve, reject) => {
        const download = (downloadUrl) => {
            const file = fs.createWriteStream(filePath);

            https.get(downloadUrl, (response) => {
                // Handle redirects
                if (response.statusCode === 301 || response.statusCode === 302) {
                    const redirectUrl = response.headers.location;
                    if (redirectUrl) {
                        console.log(`Following redirect to: ${redirectUrl}`);
                        file.close();
                        fs.unlink(filePath, () => { }); // Delete partially downloaded file
                        download(redirectUrl);
                        return;
                    }
                }

                if (response.statusCode !== 200) {
                    reject(new Error(`Download failed, status code: ${response.statusCode}`));
                    return;
                }

                // Show download progress
                const totalSize = parseInt(response.headers['content-length'], 10);
                let downloadedSize = 0;

                response.on('data', (chunk) => {
                    downloadedSize += chunk.length;
                    if (totalSize > 0) {
                        const progress = (downloadedSize / totalSize * 100).toFixed(1);
                        console.log(`Download progress: ${progress}% (${downloadedSize}/${totalSize} bytes)`);

                        // Send progress update to frontend
                        if (progressCallback) {
                            progressCallback({
                                progress: parseFloat(progress),
                                downloaded: downloadedSize,
                                total: totalSize
                            });
                        }
                    }
                });

                response.pipe(file);

                file.on('finish', () => {
                    file.close();
                    console.log('File download completed');
                    resolve();
                });

                file.on('error', (err) => {
                    fs.unlink(filePath, () => { }); // Delete partially downloaded file
                    reject(err);
                });
            }).on('error', (err) => {
                reject(err);
            });
        };

        download(url);
    });
}

// Function to extract tar.gz files
function extractTarGz(tarGzPath, extractPath) {
    return new Promise((resolve, reject) => {
        // Ensure extraction directory exists
        if (!fs.existsSync(extractPath)) {
            fs.mkdirSync(extractPath, { recursive: true });
        }

        tar.extract({
            file: tarGzPath,
            cwd: extractPath
        }).then(() => {
            resolve();
        }).catch((err) => {
            reject(err);
        });
    });
}

// Function to extract zip files
function extractZip(zipPath, extractPath) {
    return new Promise((resolve, reject) => {
        try {
            // Ensure extraction directory exists
            if (!fs.existsSync(extractPath)) {
                fs.mkdirSync(extractPath, { recursive: true });
            }

            const zip = new AdmZip(zipPath);
            zip.extractAllTo(extractPath, true);
            resolve();
        } catch (err) {
            reject(err);
        }
    });
}

const createWindow = () => {
    mainWindow = new BrowserWindow({
        width: 530,
        height: 380,
        resizable: false,
        webPreferences: {
            nodeIntegration: true,
            contextIsolation: false
        },
        icon: path.join(__dirname, 'images/icon.png')
    });

    // const menuTemplate = [];
    // const menu = Menu.buildFromTemplate(menuTemplate);
    // Menu.setApplicationMenu(menu);

    mainWindow.loadFile('login.html');
}

// Handle opening settings page
ipcMain.on('open-settings', () => {
    if (mainWindow) {
        // Check connection type from localStorage (we need to read it from the renderer process)
        // For now, we'll check if we can determine the mode by checking if remote connection info exists
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const versionFile = path.join(downloadDir, 'version.txt');

        // Check if this is a remote connection by looking at the current window's localStorage
        // Since we can't directly access localStorage from main process, we'll send a message to get the connection type
        mainWindow.webContents.executeJavaScript(`
            localStorage.getItem('type') || 'local'
        `).then((connectionType) => {
            if (connectionType === 'remote') {
                // Remote mode - no need to start CLIProxyAPI process
                console.log('Remote mode detected, skipping CLIProxyAPI process startup');

                // Close current window and create settings window
                mainWindow.close();

                // Create new window for settings
                const settingsWindow = new BrowserWindow({
                    width: 930,
                    height: 600,
                    resizable: false,
                    webPreferences: {
                        nodeIntegration: true,
                        contextIsolation: false
                    },
                    icon: path.join(__dirname, 'images/icon.png')
                });

                settingsWindow.loadFile('settings.html');

                // Update mainWindow reference to the new window
                mainWindow = settingsWindow;
            } else {
                // Local mode - start CLIProxyAPI process
                console.log('Local mode detected, starting CLIProxyAPI process');

                if (fs.existsSync(versionFile)) {
                    const currentVersion = fs.readFileSync(versionFile, 'utf8').trim();
                    const versionPath = path.join(downloadDir, currentVersion);

                    // Check if password is set
                    const secretKeyResult = checkSecretKey();
                    if (secretKeyResult.needsPassword) {
                        console.log('Password not set, cannot start CLIProxyAPI');
                        // Send error message to frontend, do not navigate to settings page
                        mainWindow.webContents.send('process-start-error', {
                            error: 'Password not set, cannot start CLIProxyAPI',
                            reason: secretKeyResult.reason
                        });
                        return;
                    }

                    const startResult = startCLIProxyAPI(versionPath);
                    if (startResult.success) {
                        console.log(`CLIProxyAPI process started successfully, PID: ${startResult.pid}`);

                        // Start monitoring process
                        startProcessMonitor();

                        // Process started successfully, close current window and create settings window
                        mainWindow.close();

                        // Create new window for settings
                        const settingsWindow = new BrowserWindow({
                            width: 930,
                            height: 600,
                            resizable: false,
                            webPreferences: {
                                nodeIntegration: true,
                                contextIsolation: false
                            },
                            icon: path.join(__dirname, 'images/icon.png')
                        });

                        settingsWindow.loadFile('settings.html');

                        // Update mainWindow reference to the new window
                        mainWindow = settingsWindow;
                    } else {
                        console.error('CLIProxyAPI process start failed:', startResult.error);
                        // Send error message to frontend, do not navigate to settings page
                        mainWindow.webContents.send('process-start-error', {
                            error: 'CLIProxyAPI process start failed',
                            reason: startResult.error
                        });
                    }
                } else {
                    console.error('Version file does not exist, cannot start CLIProxyAPI');
                    // Send error message to frontend, do not navigate to settings page
                    mainWindow.webContents.send('process-start-error', {
                        error: 'Version file does not exist, cannot start CLIProxyAPI',
                        reason: 'Version file does not exist'
                    });
                }
            }
        }).catch((error) => {
            console.error('Error determining connection type:', error);
            // Fallback to local mode behavior
            console.log('Fallback to local mode behavior');

            if (fs.existsSync(versionFile)) {
                const currentVersion = fs.readFileSync(versionFile, 'utf8').trim();
                const versionPath = path.join(downloadDir, currentVersion);

                // Check if password is set
                const secretKeyResult = checkSecretKey();
                if (secretKeyResult.needsPassword) {
                    console.log('Password not set, cannot start CLIProxyAPI');
                    // Send error message to frontend, do not navigate to settings page
                    mainWindow.webContents.send('process-start-error', {
                        error: 'Password not set, cannot start CLIProxyAPI',
                        reason: secretKeyResult.reason
                    });
                    return;
                }

                const startResult = startCLIProxyAPI(versionPath);
                if (startResult.success) {
                    console.log(`CLIProxyAPI process started successfully, PID: ${startResult.pid}`);

                    // Start monitoring process
                    startProcessMonitor();

                    // Process started successfully, close current window and create settings window
                    mainWindow.close();

                    // Create new window for settings
                    const settingsWindow = new BrowserWindow({
                        width: 930,
                        height: 600,
                        resizable: false,
                        webPreferences: {
                            nodeIntegration: true,
                            contextIsolation: false
                        },
                        icon: path.join(__dirname, 'images/icon.png')
                    });

                    settingsWindow.loadFile('settings.html');

                    // Update mainWindow reference to the new window
                    mainWindow = settingsWindow;
                } else {
                    console.error('CLIProxyAPI process start failed:', startResult.error);
                    // Send error message to frontend, do not navigate to settings page
                    mainWindow.webContents.send('process-start-error', {
                        error: 'CLIProxyAPI process start failed',
                        reason: startResult.error
                    });
                }
            } else {
                console.error('Version file does not exist, cannot start CLIProxyAPI');
                // Send error message to frontend, do not navigate to settings page
                mainWindow.webContents.send('process-start-error', {
                    error: 'Version file does not exist, cannot start CLIProxyAPI',
                    reason: 'Version file does not exist'
                });
            }
        });
    }
});

// Handle returning to login page
ipcMain.on('return-to-login', () => {
    if (mainWindow) {
        // Check connection type to determine if we need to stop the process
        mainWindow.webContents.executeJavaScript(`
            localStorage.getItem('type') || 'local'
        `).then((connectionType) => {
            if (connectionType === 'local') {
                // Local mode - stop process monitoring and CLIProxyAPI process
                console.log('Local mode detected, stopping CLIProxyAPI process');
                stopProcessMonitor();
                stopCLIProxyAPI();
            } else {
                // Remote mode - no local process to stop
                console.log('Remote mode detected, no local process to stop');
            }

            // Close current window
            mainWindow.close();

            // Recreate login window
            createWindow();
        }).catch((error) => {
            console.error('Error determining connection type:', error);
            // Fallback to stopping process (local mode behavior)
            console.log('Fallback to local mode behavior - stopping process');
            stopProcessMonitor();
            stopCLIProxyAPI();

            // Close current window
            mainWindow.close();

            // Recreate login window
            createWindow();
        });
    }
});

// Handle restarting CLIProxyAPI
ipcMain.on('restart-cliproxyapi', () => {
    console.log('Received CLIProxyAPI restart request');

    // Check connection type to determine if we can restart the process
    if (mainWindow) {
        mainWindow.webContents.executeJavaScript(`
            localStorage.getItem('type') || 'local'
        `).then((connectionType) => {
            if (connectionType === 'local') {
                // Local mode - restart CLIProxyAPI process
                console.log('Local mode detected, restarting CLIProxyAPI process');
                restartCLIProxyAPI();
            } else {
                // Remote mode - no local process to restart
                console.log('Remote mode detected, no local process to restart');
                if (mainWindow) {
                    mainWindow.webContents.send('cliproxyapi-restart-failed', {
                        success: false,
                        error: 'Cannot restart process in remote mode'
                    });
                }
            }
        }).catch((error) => {
            console.error('Error determining connection type:', error);
            // Fallback to restarting process (local mode behavior)
            console.log('Fallback to local mode behavior - restarting process');
            restartCLIProxyAPI();
        });
    } else {
        // No main window, fallback to restarting process
        restartCLIProxyAPI();
    }
});

// Handle checking version and downloading CLIProxyAPI
ipcMain.handle('check-version-and-download', async () => {
    try {
        // Send start check status
        if (mainWindow) {
            mainWindow.webContents.send('download-status', { status: 'checking' });
        }

        const result = await checkVersionAndUpdate();

        // Send check completion status
        if (mainWindow) {
            if (result.success) {
                if (result.needsUpdate) {
                    // Update needed, send update prompt
                    mainWindow.webContents.send('download-status', {
                        status: 'update-available',
                        currentVersion: result.version,
                        latestVersion: result.latestVersion,
                        path: result.path
                    });
                } else {
                    // Version is latest, complete directly
                    mainWindow.webContents.send('download-status', {
                        status: 'latest',
                        version: result.version,
                        path: result.path
                    });
                }
            } else {
                mainWindow.webContents.send('download-status', {
                    status: 'failed',
                    error: result.error
                });
            }
        }

        return result;
    } catch (error) {
        console.error('Error checking version:', error);

        // Send failure status
        if (mainWindow) {
            mainWindow.webContents.send('download-status', {
                status: 'failed',
                error: error.message
            });
        }

        return { success: false, error: error.message };
    }
});

// Handle downloading CLIProxyAPI (for update confirmation)
ipcMain.handle('download-cliproxyapi', async () => {
    try {
        // Send start download status
        if (mainWindow) {
            mainWindow.webContents.send('download-status', { status: 'starting' });
        }

        // Get latest version information
        const releaseInfo = await getLatestReleaseInfo();
        const version = releaseInfo.tag_name.replace(/^v/, '');

        const result = await downloadAndExtractCLIProxyAPI(version, releaseInfo);

        // Send completion status
        if (mainWindow) {
            if (result.success) {
                mainWindow.webContents.send('download-status', {
                    status: 'completed',
                    version: result.version,
                    path: result.path
                });
            } else {
                mainWindow.webContents.send('download-status', {
                    status: 'failed',
                    error: result.error
                });
            }
        }

        return result;
    } catch (error) {
        console.error('Error downloading CLIProxyAPI:', error);

        // Send failure status
        if (mainWindow) {
            mainWindow.webContents.send('download-status', {
                status: 'failed',
                error: error.message
            });
        }

        return { success: false, error: error.message };
    }
});

// Handle checking secret key
ipcMain.handle('check-secret-key', () => {
    try {
        const result = checkSecretKey();
        return result;
    } catch (error) {
        console.error('Error checking secret-key:', error);
        return { needsPassword: true, reason: 'Check failed' };
    }
});

// Handle updating secret key
ipcMain.handle('update-secret-key', (event, secretKey) => {
    try {
        const result = updateSecretKey(secretKey);
        return result;
    } catch (error) {
        console.error('Error updating secret-key:', error);
        return { success: false, error: error.message };
    }
});

// Handle reading config.yaml file
ipcMain.handle('read-config-yaml', () => {
    try {
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const configPath = path.join(downloadDir, 'config.yaml');

        if (!fs.existsSync(configPath)) {
            console.log('config.yaml file does not exist');
            return {};
        }

        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent);
        return config || {};
    } catch (error) {
        console.error('Failed to read config.yaml:', error);
        return {};
    }
});

// Handle updating config.yaml file
ipcMain.handle('update-config-yaml', (event, { endpoint, value, isDelete }) => {
    try {
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const configPath = path.join(downloadDir, 'config.yaml');

        if (!fs.existsSync(configPath)) {
            console.log('config.yaml file does not exist');
            return { success: false, error: 'Configuration file does not exist' };
        }

        // Read current configuration
        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent) || {};

        // Update configuration
        if (isDelete) {
            // Delete configuration item
            if (endpoint.includes('quota-exceeded')) {
                const setting = endpoint.split('/').pop();
                if (config['quota-exceeded'] && config['quota-exceeded'][setting] !== undefined) {
                    delete config['quota-exceeded'][setting];
                }
            } else if (endpoint.includes('remote-management')) {
                const setting = endpoint.split('.').pop();
                if (config['remote-management'] && config['remote-management'][setting] !== undefined) {
                    delete config['remote-management'][setting];
                }
            } else {
                const key = endpoint.split('/').pop();
                if (config[key] !== undefined) {
                    delete config[key];
                }
            }
        } else {
            // Update configuration item
            if (endpoint.includes('quota-exceeded')) {
                const setting = endpoint.split('/').pop();
                if (!config['quota-exceeded']) {
                    config['quota-exceeded'] = {};
                }
                config['quota-exceeded'][setting] = value;
            } else if (endpoint.includes('remote-management')) {
                const setting = endpoint.split('.').pop();
                if (!config['remote-management']) {
                    config['remote-management'] = {};
                }
                config['remote-management'][setting] = value;
            } else {
                const key = endpoint.split('/').pop();
                config[key] = value;
            }
        }

        // Write back to file
        const updatedContent = yaml.dump(config, {
            indent: 2,
            lineWidth: -1,
            noRefs: true
        });

        fs.writeFileSync(configPath, updatedContent, 'utf8');
        console.log(`Updated ${endpoint} in config.yaml`);
        return { success: true };
    } catch (error) {
        console.error('Failed to update config.yaml:', error);
        return { success: false, error: error.message };
    }
});

// Handle reading auth files from local directory
ipcMain.handle('read-local-auth-files', () => {
    try {
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const configPath = path.join(downloadDir, 'config.yaml');

        if (!fs.existsSync(configPath)) {
            console.log('config.yaml file does not exist');
            return [];
        }

        // Read configuration file to get auth-dir
        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent) || {};

        const authDir = config['auth-dir'];
        if (!authDir) {
            console.log('auth-dir not configured in config.yaml');
            return [];
        }

        // Parse auth-dir path (supports ~, relative paths and absolute paths)
        const authDirPath = resolvePath(authDir, path.dirname(configPath));

        if (!fs.existsSync(authDirPath)) {
            console.log(`Authentication file directory does not exist: ${authDirPath}`);
            return [];
        }

        // Read files in directory
        const files = fs.readdirSync(authDirPath);
        const authFiles = [];

        for (const file of files) {
            const filePath = path.join(authDirPath, file);
            const stats = fs.statSync(filePath);

            if (stats.isFile()) {
                // Only process JSON files
                if (file.toLowerCase().endsWith('.json')) {
                    let fileType = 'unknown';

                    try {
                        // Read JSON file content to get type field
                        const fileContent = fs.readFileSync(filePath, 'utf8');
                        const jsonData = JSON.parse(fileContent);

                        // If JSON has type field, use it; otherwise use 'unknown'
                        if (jsonData && typeof jsonData.type === 'string') {
                            fileType = jsonData.type;
                        }
                    } catch (error) {
                        // If JSON parsing fails, keep 'unknown'
                        console.log(`Failed to parse JSON file ${file}:`, error.message);
                    }

                    authFiles.push({
                        name: file,
                        size: stats.size,
                        modtime: stats.mtime.toISOString(),
                        type: fileType
                    });
                }
            }
        }

        return authFiles;
    } catch (error) {
        console.error('Failed to read local authentication files:', error);
        return [];
    }
});

// Handle uploading files to local auth directory
ipcMain.handle('upload-local-auth-files', (event, files) => {
    try {
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const configPath = path.join(downloadDir, 'config.yaml');

        if (!fs.existsSync(configPath)) {
            return { success: false, error: 'Configuration file does not exist' };
        }

        // Read configuration file to get auth-dir
        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent) || {};

        const authDir = config['auth-dir'];
        if (!authDir) {
            return { success: false, error: 'auth-dir not configured in config.yaml' };
        }

        // Parse auth-dir path (supports ~, relative paths and absolute paths)
        const authDirPath = resolvePath(authDir, path.dirname(configPath));

        // Ensure directory exists
        if (!fs.existsSync(authDirPath)) {
            fs.mkdirSync(authDirPath, { recursive: true });
        }

        let successCount = 0;
        let errorCount = 0;
        const errors = [];

        for (const file of files) {
            try {
                const fileName = file.name;
                const filePath = path.join(authDirPath, fileName);

                // Check if file already exists
                if (fs.existsSync(filePath)) {
                    errors.push(`${fileName}: File already exists`);
                    errorCount++;
                    continue;
                }

                // Write file
                fs.writeFileSync(filePath, file.content);
                successCount++;
            } catch (error) {
                console.error(`Failed to upload file ${file.name}:`, error);
                errors.push(`${file.name}: ${error.message}`);
                errorCount++;
            }
        }

        return {
            success: successCount > 0,
            successCount,
            errorCount,
            errors: errors.length > 0 ? errors : undefined
        };
    } catch (error) {
        console.error('Failed to upload local authentication files:', error);
        return { success: false, error: error.message };
    }
});

// Handle deleting files from local auth directory
ipcMain.handle('delete-local-auth-files', (event, filenames) => {
    try {
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const configPath = path.join(downloadDir, 'config.yaml');

        if (!fs.existsSync(configPath)) {
            return { success: false, error: 'Configuration file does not exist' };
        }

        // Read configuration file to get auth-dir
        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent) || {};

        const authDir = config['auth-dir'];
        if (!authDir) {
            return { success: false, error: 'auth-dir not configured in config.yaml' };
        }

        // Parse auth-dir path (supports ~, relative paths and absolute paths)
        const authDirPath = resolvePath(authDir, path.dirname(configPath));

        if (!fs.existsSync(authDirPath)) {
            return { success: false, error: 'Authentication file directory does not exist' };
        }

        let successCount = 0;
        let errorCount = 0;

        for (const filename of filenames) {
            try {
                const filePath = path.join(authDirPath, filename);

                if (fs.existsSync(filePath)) {
                    fs.unlinkSync(filePath);
                    successCount++;
                } else {
                    errorCount++;
                }
            } catch (error) {
                console.error(`Failed to delete file ${filename}:`, error);
                errorCount++;
            }
        }

        return {
            success: successCount > 0,
            successCount,
            errorCount
        };
    } catch (error) {
        console.error('Failed to delete local authentication files:', error);
        return { success: false, error: error.message };
    }
});

// Handle downloading files from local auth directory
ipcMain.handle('download-local-auth-files', (event, filenames) => {
    try {
        const downloadDir = path.join(os.homedir(), 'cliproxyapi');
        const configPath = path.join(downloadDir, 'config.yaml');

        if (!fs.existsSync(configPath)) {
            return { success: false, error: 'Configuration file does not exist' };
        }

        // Read configuration file to get auth-dir
        const configContent = fs.readFileSync(configPath, 'utf8');
        const config = yaml.load(configContent) || {};

        const authDir = config['auth-dir'];
        if (!authDir) {
            return { success: false, error: 'auth-dir not configured in config.yaml' };
        }

        // Parse auth-dir path (supports ~, relative paths and absolute paths)
        const authDirPath = resolvePath(authDir, path.dirname(configPath));

        if (!fs.existsSync(authDirPath)) {
            return { success: false, error: 'Authentication file directory does not exist' };
        }

        const files = [];
        let errorCount = 0;

        for (const filename of filenames) {
            try {
                const filePath = path.join(authDirPath, filename);

                if (fs.existsSync(filePath)) {
                    const content = fs.readFileSync(filePath, 'utf8');
                    files.push({
                        name: filename,
                        content: content
                    });
                } else {
                    errorCount++;
                }
            } catch (error) {
                console.error(`Failed to read file ${filename}:`, error);
                errorCount++;
            }
        }

        return {
            success: files.length > 0,
            files,
            errorCount
        };
    } catch (error) {
        console.error('Failed to download local authentication files:', error);
        return { success: false, error: error.message };
    }
});

app.whenReady().then(() => {
    createWindow()
});

// Clean up CLIProxyAPI process when application exits
app.on('before-quit', () => {
    console.log('Application is about to exit, cleaning up CLIProxyAPI process...');
    stopProcessMonitor();
    stopCLIProxyAPI();
});

app.on('window-all-closed', () => {
    console.log('All windows closed, cleaning up CLIProxyAPI process...');
    stopProcessMonitor();
    stopCLIProxyAPI();
    app.quit();
});