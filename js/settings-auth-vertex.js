// Vertex credential import flow

function showVertexImportDialog() {
    const modal = document.createElement('div');
    modal.className = 'modal show';
    modal.id = 'vertex-import-modal';
    modal.innerHTML = `
        <div class="modal-content">
            <div class="modal-header">
                <h3 class="modal-title">Vertex Credential Import</h3>
                <button class="modal-close" id="vertex-modal-close">&times;</button>
            </div>
            <div class="modal-body">
                <div class="codex-auth-content">
                    <p>Upload a Google service account JSON and optional Vertex location.</p>
                    <div class="form-group">
                        <label for="vertex-file-input">Service Account JSON <span class="required">*</span></label>
                        <input type="file" id="vertex-file-input" class="form-input" accept=".json">
                        <small class="form-help">The file name must end with .json.</small>
                    </div>
                    <div class="form-group">
                        <label for="vertex-location-input">Location</label>
                        <input type="text" id="vertex-location-input" class="form-input" placeholder="us-central1" value="us-central1">
                        <small class="form-help">Defaults to us-central1 when empty.</small>
                    </div>
                    <div class="auth-actions">
                        <button type="button" id="vertex-import-btn" class="btn-primary">Import</button>
                        <button type="button" id="vertex-cancel-btn" class="btn-cancel">Cancel</button>
                    </div>
                </div>
            </div>
        </div>`;
    document.body.appendChild(modal);

    const fileInput = document.getElementById('vertex-file-input');
    const locationInput = document.getElementById('vertex-location-input');
    const importBtn = document.getElementById('vertex-import-btn');

    document.getElementById('vertex-modal-close').addEventListener('click', closeVertexImportDialog);
    document.getElementById('vertex-cancel-btn').addEventListener('click', closeVertexImportDialog);
    importBtn.addEventListener('click', () => handleVertexImport(fileInput, locationInput, importBtn));
    document.addEventListener('keydown', handleVertexEscapeKey);

    if (fileInput) {
        fileInput.focus();
    }
}

function handleVertexEscapeKey(e) {
    if (e.key === 'Escape') {
        closeVertexImportDialog();
    }
}

async function handleVertexImport(fileInput, locationInput, importBtn) {
    try {
        const files = fileInput && fileInput.files ? Array.from(fileInput.files) : [];
        if (files.length === 0) {
            showError('Please select a service account JSON file');
            return;
        }

        const file = files[0];
        if (!file.name.toLowerCase().endsWith('.json')) {
            showError('Service account file must be a .json file');
            return;
        }

        const location = locationInput && locationInput.value ? locationInput.value.trim() : '';
        const resolvedLocation = location || 'us-central1';

        importBtn.disabled = true;
        importBtn.textContent = 'Importing...';

        const result = await configManager.importVertexCredential(file, resolvedLocation);

        if (result && result.success) {
            const project = result.data?.project_id ? ` for ${result.data.project_id}` : '';
            const locText = result.data?.location ? ` (${result.data.location})` : '';
            showSuccessMessage(`Vertex credential imported${project}${locText}`);
            closeVertexImportDialog();
            if (typeof loadAuthFiles === 'function') {
                await loadAuthFiles();
            }
        } else {
            showError(result?.error || 'Failed to import Vertex credential');
        }
    } catch (error) {
        console.error('Error importing Vertex credential:', error);
        showError('Failed to import Vertex credential: ' + error.message);
    } finally {
        if (importBtn) {
            importBtn.disabled = false;
            importBtn.textContent = 'Import';
        }
    }
}

function closeVertexImportDialog() {
    document.removeEventListener('keydown', handleVertexEscapeKey);
    const modal = document.getElementById('vertex-import-modal');
    if (modal) {
        modal.remove();
    }
}
