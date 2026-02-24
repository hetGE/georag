// geoRAG Processing Status Polling

let pollInterval = null;

document.addEventListener('DOMContentLoaded', () => {
    document.getElementById('stop-processing-btn')?.addEventListener('click', stopProcessing);
    startPolling();
});

function startPolling() {
    if (pollInterval) return;
    pollInterval = setInterval(checkProcessingStatus, 2000);
}

async function checkProcessingStatus() {
    try {
        const status = await apiGet('/api/processing/status');
        const container = document.getElementById('progress-bar-container');
        const statusEl = document.getElementById('processing-status');

        if (status.is_running) {
            container.style.display = 'flex';
            const pct = status.total_files > 0
                ? Math.round((status.processed_files / status.total_files) * 100)
                : 0;

            document.getElementById('progress-bar').value = pct;
            document.getElementById('progress-text').textContent =
                `${status.processed_files} / ${status.total_files}` +
                (status.current_file ? ` — ${status.current_file}` : '');

            statusEl.textContent = `Processing... ${pct}%`;

            if (status.failed_files > 0) {
                statusEl.textContent += ` (${status.failed_files} failed)`;
            }
        } else {
            container.style.display = 'none';
            statusEl.textContent = '';
        }
    } catch {
        // API not available, ignore
    }
}

async function stopProcessing() {
    await apiPost('/api/processing/stop');
    document.getElementById('processing-status').textContent = 'Stopping...';
}
