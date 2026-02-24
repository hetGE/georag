// geoRAG Library Panel Controller

const LibraryPanel = (() => {
    let isOpen = false;
    let pollInterval = null;
    let lastStatusJSON = '';
    const POLL_MS = 2000;
    const activePage = document.body.dataset.activePage;

    let panelEl, toggleBtnEl, contentEl, badgeEl;

    function init() {
        panelEl     = document.getElementById('library-panel');
        toggleBtnEl = document.getElementById('library-panel-toggle');
        contentEl   = document.getElementById('library-panel-content');
        badgeEl     = document.getElementById('library-panel-badge');

        toggleBtnEl.addEventListener('click', toggle);

        // Restore state
        const saved = localStorage.getItem('libraryPanelOpen');
        if (saved === 'true') {
            openPanel(false);
        }

        checkStatus().then(() => {
            // Auto-open on first visit if not_started
            if (saved === null && lastStatusJSON) {
                try {
                    const s = JSON.parse(lastStatusJSON);
                    if (s.phase === 'not_started') openPanel();
                } catch {}
            }
        });
        startPolling();
    }

    function toggle() { isOpen ? closePanel() : openPanel(); }

    function openPanel(animate = true) {
        isOpen = true;
        if (!animate) panelEl.classList.add('no-transition');
        panelEl.classList.add('open');
        if (!animate) {
            panelEl.offsetHeight; // force reflow
            panelEl.classList.remove('no-transition');
        }
        localStorage.setItem('libraryPanelOpen', 'true');
    }

    function closePanel() {
        isOpen = false;
        panelEl.classList.remove('open');
        localStorage.setItem('libraryPanelOpen', 'false');
    }

    // --- Polling ---

    async function checkStatus() {
        try {
            const status = await apiGet('/api/processing/onboarding-status');
            const json = JSON.stringify(status);
            if (json !== lastStatusJSON) {
                lastStatusJSON = json;
                renderPanel(status);
            }
            updateBadge(status);
            const wasProcessing = window.libraryIsProcessing;
            window.libraryIsProcessing = status.is_processing || status.phase === 'processing';
            if (wasProcessing !== window.libraryIsProcessing) {
                window.updateChatInputState?.();
            }
        } catch {
            // API not available
        }
    }

    function startPolling() {
        if (pollInterval) return;
        pollInterval = setInterval(checkStatus, POLL_MS);
    }

    // --- Badge ---

    function updateBadge(status) {
        if (status.phase === 'stopping') {
            badgeEl.textContent = '…';
            badgeEl.style.display = '';
        } else if (status.phase === 'processing') {
            const pct = status.total_files > 0
                ? Math.round((status.processed_files / status.total_files) * 100) : 0;
            badgeEl.textContent = pct + '%';
            badgeEl.style.display = '';
        } else if (status.phase === 'not_started' || status.phase === 'scanned') {
            badgeEl.textContent = '!';
            badgeEl.style.display = '';
        } else {
            badgeEl.style.display = 'none';
        }
    }

    // --- Rendering ---

    function renderPanel(status) {
        let html = `<div class="lp-header">
            <h3>Library</h3>
            <button class="lp-close" id="lp-close-btn" aria-label="Close">&times;</button>
        </div>`;

        if (status.phase === 'not_started') {
            html += renderScanPhase();
        } else if (status.phase === 'scanned') {
            html += renderScannedPhase(status);
        } else if (status.phase === 'stopping') {
            html += renderStoppingPhase(status);
        } else if (status.phase === 'processing') {
            html += renderProcessingPhase(status);
        } else if (status.phase === 'complete') {
            html += renderCompletePhase(status);
        }

        // Action buttons (only when not processing/stopping)
        if (status.phase !== 'not_started' && status.phase !== 'stopping' && !status.is_processing) {
            html += renderActionButtons(status);
        }

        contentEl.innerHTML = html;
        bindEvents(status);
    }

    function renderScanPhase() {
        return `<div class="lp-phase">
            <p>Let's index your geotechnical document library. Scan your Engineering folder to discover all documents.</p>
            <div class="lp-actions">
                <button id="lp-scan-btn">Scan Library</button>
            </div>
        </div>`;
    }

    function renderScannedPhase(status) {
        const extSummary = formatExtSummary(status.by_extension || {});
        const remaining = status.new_files;
        const already = status.processed_files;
        const label = already > 0 ? 'Resume Processing' : 'Start Processing';

        return `<div class="lp-phase">
            <p>Found <strong>${status.total_files.toLocaleString()}</strong> files${extSummary}.</p>
            <p>${remaining.toLocaleString()} files to process. We'll extract text, auto-tag by topic, and build the search index. You can stop and resume anytime.</p>
            ${already > 0 ? `<p class="lp-stats">${already.toLocaleString()} already processed.</p>` : ''}
            <div class="lp-actions">
                <button id="lp-process-btn">${label}</button>
                <button id="lp-skip-btn" class="outline secondary">Dismiss</button>
            </div>
        </div>`;
    }

    function renderProcessingPhase(status) {
        const pct = status.total_files > 0
            ? Math.round((status.processed_files / status.total_files) * 100) : 0;

        const extras = [];
        if (status.failed_files > 0) extras.push(`${status.failed_files} failed`);
        if (status.skipped_files > 0) extras.push(`${status.skipped_files} skipped`);

        return `<div class="lp-phase">
            <progress value="${pct}" max="100"></progress>
            <p class="lp-progress-text">${status.processed_files.toLocaleString()} / ${status.total_files.toLocaleString()} files (${pct}%)</p>
            ${extras.length ? `<p class="lp-stats">${extras.join(' &middot; ')}</p>` : ''}
            ${status.current_file ? `<p class="lp-stats" style="font-family:monospace;font-size:0.7rem;word-break:break-all;">${escapeHtml(status.current_file)}</p>` : ''}
            <p style="font-size:0.82rem;color:var(--pico-muted-color);">Progress is saved automatically. You can close this panel.</p>
            <div class="lp-actions">
                <button id="lp-stop-btn" class="outline secondary">Stop Processing</button>
            </div>
        </div>`;
    }

    function renderStoppingPhase(status) {
        const pct = status.total_files > 0
            ? Math.round((status.processed_files / status.total_files) * 100) : 0;
        return `<div class="lp-phase">
            <progress value="${pct}" max="100"></progress>
            <p class="lp-progress-text">${status.processed_files.toLocaleString()} / ${status.total_files.toLocaleString()} files (${pct}%)</p>
            <p style="font-size:0.82rem;color:var(--pico-muted-color);">Stopping after current file&hellip;</p>
            <div class="lp-actions">
                <button disabled class="outline secondary" aria-busy="true">Stopping&hellip;</button>
            </div>
        </div>`;
    }

    function renderCompletePhase(status) {
        return `<div class="lp-phase">
            <p><strong>${status.processed_files.toLocaleString()}</strong> files processed${status.failed_files > 0 ? `, ${status.failed_files} failed` : ''}. Your library is ready for searching.</p>
        </div>`;
    }

    function renderActionButtons(status) {
        return `<hr class="lp-divider">
        <div class="lp-phase">
            <p class="lp-section-label">Actions</p>
            <div class="lp-actions">
                <button id="lp-action-scan" class="outline">Scan Files</button>
                ${status.phase === 'complete' ? `<button id="lp-action-process-new" class="outline">Process New Files</button>` : ''}
            </div>
        </div>`;
    }

    // --- Event Binding ---

    function bindEvents(status) {
        document.getElementById('lp-close-btn')
            ?.addEventListener('click', closePanel);

        document.getElementById('lp-scan-btn')
            ?.addEventListener('click', handleScan);
        document.getElementById('lp-process-btn')
            ?.addEventListener('click', handleStartProcessing);
        document.getElementById('lp-skip-btn')
            ?.addEventListener('click', handleDismiss);
        document.getElementById('lp-stop-btn')
            ?.addEventListener('click', handleStop);

        // Action buttons
        document.getElementById('lp-action-scan')
            ?.addEventListener('click', handleActionScan);
        document.getElementById('lp-action-process-new')
            ?.addEventListener('click', handleProcessNew);
    }

    // --- Action Handlers ---

    async function handleScan() {
        const btn = document.getElementById('lp-scan-btn');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');
        btn.textContent = 'Scanning...';
        try {
            await apiPost('/api/processing/scan');
            lastStatusJSON = '';
            await checkStatus();
        } catch {
            btn.textContent = 'Scan Failed — Retry';
            btn.disabled = false;
            btn.setAttribute('aria-busy', 'false');
        }
    }

    async function handleStartProcessing() {
        if (window.chatIsStreaming?.()) return;
        const btn = document.getElementById('lp-process-btn');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');
        const result = await apiPost('/api/processing/start', {});
        if (result.error) {
            alert(result.error);
            btn.disabled = false;
            btn.setAttribute('aria-busy', 'false');
            return;
        }
        lastStatusJSON = '';
        await checkStatus();
    }

    async function handleDismiss() {
        await apiPost('/api/processing/onboarding-dismiss');
        lastStatusJSON = '';
        await checkStatus();
    }

    async function handleStop() {
        const btn = document.getElementById('lp-stop-btn');
        btn.disabled = true;
        btn.textContent = 'Stopping\u2026';
        await apiPost('/api/processing/stop');
        lastStatusJSON = '';
        await checkStatus();
    }

    async function handleActionScan() {
        const btn = document.getElementById('lp-action-scan');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');
        btn.textContent = 'Scanning...';
        try {
            const result = await apiPost('/api/processing/scan');
            btn.textContent = `Found ${result.files_found?.toLocaleString() || 0} files`;
            window.documentsRefresh?.();
            lastStatusJSON = '';
            await checkStatus();
        } catch {
            btn.textContent = 'Scan Failed';
        }
        btn.setAttribute('aria-busy', 'false');
        setTimeout(() => {
            lastStatusJSON = '';
            checkStatus();
        }, 2000);
    }

    async function handleProcessNew() {
        if (window.chatIsStreaming?.()) return;
        const btn = document.getElementById('lp-action-process-new');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');
        const result = await apiPost('/api/processing/start', {});
        if (result.error) {
            alert(result.error);
            btn.disabled = false;
            btn.setAttribute('aria-busy', 'false');
            return;
        }
        lastStatusJSON = '';
        await checkStatus();
    }

    // --- Utility ---

    function formatExtSummary(byExt) {
        const entries = Object.entries(byExt);
        if (!entries.length) return '';
        const parts = entries.slice(0, 5).map(([ext, count]) => {
            const label = ext ? ext.toUpperCase() : 'other';
            return `${count.toLocaleString()} ${label}`;
        });
        return ' (' + parts.join(', ') + ')';
    }

    return { init, openPanel, closePanel, toggle, checkStatus };
})();

document.addEventListener('DOMContentLoaded', () => {
    LibraryPanel.init();
});
