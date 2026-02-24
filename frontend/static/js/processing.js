// geoRAG Processing Status Polling

let pollInterval = null;

const ACTION_BTN_IDS = ['scan-btn', 'process-btn', 'autotag-btn', 'process-new-btn'];
const PROCESSING_EL_IDS = ['processing-status', 'progress-bar', 'stop-btn'];

document.addEventListener('DOMContentLoaded', () => {
    document.getElementById('stop-btn')?.addEventListener('click', stopProcessing);
    checkProcessingStatus().then(startPolling);
});

function startPolling() {
    if (pollInterval) return;
    pollInterval = setInterval(checkProcessingStatus, 2000);
}

async function checkProcessingStatus() {
    try {
        const status = await apiGet('/api/processing/status');
        const statusEl = document.getElementById('processing-status');

        if (status.is_running) {
            const pct = status.total_files > 0
                ? Math.round((status.processed_files / status.total_files) * 100)
                : 0;

            document.getElementById('progress-bar').value = pct;

            statusEl.textContent = `Processing... ${status.processed_files} / ${status.total_files} files (${pct}%)`
                + (status.failed_files > 0 ? ` · ${status.failed_files} failed` : '')
                + (status.skipped_files > 0 ? ` · ${status.skipped_files} skipped` : '');

            // Hide action buttons, show processing elements
            ACTION_BTN_IDS.forEach(id => {
                document.getElementById(id).style.display = 'none';
            });
            PROCESSING_EL_IDS.forEach(id => {
                document.getElementById(id).style.display = id === 'progress-bar' ? '' : 'inline-block';
            });
        } else {
            statusEl.textContent = '';

            // Show action buttons, hide processing elements
            ACTION_BTN_IDS.forEach(id => {
                document.getElementById(id).style.display = '';
            });
            PROCESSING_EL_IDS.forEach(id => {
                document.getElementById(id).style.display = 'none';
            });
        }
    } catch {
        // API not available, ignore
    }
}

async function stopProcessing() {
    await apiPost('/api/processing/stop');
    document.getElementById('processing-status').textContent = 'Stopping...';
}
