// geoRAG Library Panel Controller

const LibraryPanel = (() => {
    let isOpen = false;
    let pollInterval = null;
    let lastStatusJSON = '';
    let lastStreamingState = false;
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

        // Cross-tab sync: another tab started processing → force immediate status check
        window.syncChannel.addEventListener('message', (e) => {
            if (e.data.type === 'processing-started' || e.data.type === 'tags-changed') {
                lastStatusJSON = '';
                checkStatus();
            }
        });
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

    let lastErrorCount = 0;

    async function checkStatus() {
        try {
            const status = await apiGet('/api/processing/onboarding-status');
            const json = JSON.stringify(status);
            const streamingNow = window.chatIsStreaming?.() || false;
            if (json !== lastStatusJSON || streamingNow !== lastStreamingState) {
                lastStatusJSON = json;
                lastStreamingState = streamingNow;
                renderPanel(status);
            }
            updateBadge(status);
            const wasProcessing = window.libraryIsProcessing;
            window.libraryIsProcessing = status.is_processing || status.phase === 'processing'
                || status.phase === 'exploring' || status.phase === 'explore_stopping'
                || status.phase === 'ocr_processing' || status.phase === 'ocr_stopping';
            if (wasProcessing !== window.libraryIsProcessing) {
                window.updateChatInputState?.();
            }

            // Detect new llama-server errors during processing/explore
            const errors = status.errors || [];
            if (errors.length > lastErrorCount) {
                const newErrors = errors.slice(lastErrorCount);
                const llmError = newErrors.find(e =>
                    /connect|timeout|ConnectError|embed|model.*not.*loaded|No models loaded/i.test(e)
                );
                if (llmError) window.requireLlmServers();
            }
            lastErrorCount = errors.length;
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
        if (status.phase === 'stopping' || status.phase === 'explore_stopping' || status.phase === 'ocr_stopping') {
            badgeEl.textContent = '…';
            badgeEl.style.display = '';
        } else if (status.phase === 'processing') {
            const pct = status.total_files > 0
                ? Math.round((status.processed_files / status.total_files) * 100) : 0;
            badgeEl.textContent = pct + '%';
            badgeEl.style.display = '';
        } else if (status.phase === 'exploring') {
            const pct = status.explore_total > 0
                ? Math.round((status.explore_processed / status.explore_total) * 100) : 0;
            badgeEl.textContent = pct + '%';
            badgeEl.style.display = '';
        } else if (status.phase === 'ocr_processing') {
            const pct = status.ocr_total > 0
                ? Math.round((status.ocr_processed / status.ocr_total) * 100) : 0;
            badgeEl.textContent = pct + '%';
            badgeEl.style.display = '';
        } else if (status.phase === 'explore_complete') {
            badgeEl.textContent = (status.explore_candidates || []).length;
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
        } else if (status.phase === 'ocr_processing') {
            html += renderOCRPhase(status);
        } else if (status.phase === 'ocr_stopping') {
            html += renderOCRStoppingPhase(status);
        } else if (status.phase === 'exploring') {
            html += renderExploringPhase(status);
        } else if (status.phase === 'explore_stopping') {
            html += renderExploreStoppingPhase(status);
        } else if (status.phase === 'explore_complete') {
            html += renderExploreCompletePhase(status);
        } else if (status.phase === 'complete') {
            html += renderCompletePhase(status);
        }

        // Action buttons (only for complete phase — scanned has inline scan button)
        if (status.phase === 'complete') {
            html += renderActionButtons(status);
        } else if (status.phase === 'not_started' || status.phase === 'scanned') {
            // Empty / pre-processing states still need backup access so users
            // can restore from a backup instead of (re-)processing from scratch.
            html += `<hr class="lp-divider">${renderBackupActions(status)}`;
        }

        // Reprocess All — always at the bottom (except initial state and explore phases)
        if (status.phase !== 'not_started' && status.phase !== 'explore_complete') {
            const rpDisabled = (status.phase === 'processing' || status.phase === 'stopping'
                || status.phase === 'exploring' || status.phase === 'explore_stopping'
                || status.phase === 'ocr_processing' || status.phase === 'ocr_stopping'
                || window.chatIsStreaming?.()) ? ' disabled' : '';
            html += `<div class="lp-reprocess-footer">
                <button id="lp-action-reprocess" class="lp-reprocess-btn outline"${rpDisabled}>Reprocess All</button>
            </div>`;
        }

        contentEl.innerHTML = html;
        bindEvents(status);
    }

    function renderScanPhase() {
        const busy = window.chatIsStreaming?.();
        return `<div class="lp-phase">
            <p>Let's index your geotechnical document library. Scan your Engineering folder to discover all documents.</p>
            <div class="lp-actions">
                <button id="lp-scan-btn"${busy ? ' disabled' : ''}>Scan Library</button>
            </div>
        </div>`;
    }

    function renderScannedPhase(status) {
        const busy = window.chatIsStreaming?.();
        const extSummary = formatExtSummary(status.by_extension || {});
        const remaining = status.new_files;
        const already = status.processed_files;
        const label = already > 0 ? 'Resume Processing' : 'Start Processing';

        return `<div class="lp-phase">
            <p>Found <strong>${status.total_files.toLocaleString()}</strong> files${extSummary}.</p>
            <p>${remaining.toLocaleString()} ${already > 0 ? 'new files to process. You can resume processing.' : 'files to process. We\'ll extract text, auto-tag by topic, and build the search index. You can stop and resume anytime.'}</p>
            ${already > 0 ? `<p class="lp-stats">${already.toLocaleString()} already processed.</p>` : ''}
            ${status.total_tags_assigned > 0 ? `<p class="lp-stats">${status.total_tags_assigned.toLocaleString()} tags assigned.</p>` : ''}
            ${status.failed_files > 0 ? `<p class="lp-stats">${status.failed_files.toLocaleString()} can't be read (failed).</p>` : ''}
            ${status.ocr_failed_pdfs > 0 ? `<p class="lp-stats">${status.ocr_failed_pdfs.toLocaleString()} PDF${status.ocr_failed_pdfs !== 1 ? 's' : ''} still unreadable after OCR, no extractable text.</p>` : ''}
            ${status.skipped_files > 0 ? `<p class="lp-stats">${status.skipped_files.toLocaleString()} skipped.</p>` : ''}
            <div class="lp-actions">
                <button id="lp-process-btn"${busy ? ' disabled' : ''}>${label}</button>
                <button id="lp-action-scan" class="outline"${busy ? ' disabled' : ''}>Scan Files</button>
                ${already > 0 ? `<button id="lp-action-explore" class="outline"${busy ? ' disabled' : ''}>Explore New Tags</button>` : ''}
            </div>
        </div>`;
    }

    function renderProcessingPhase(status) {
        const pct = status.total_files > 0
            ? Math.round((status.processed_files / status.total_files) * 100) : 0;

        const extras = [];
        if (status.failed_files > 0) extras.push(`${status.failed_files} failed`);

        const tagLine = (status.new_tags_added > 0)
            ? `<p class="lp-stats">${status.new_tags_added} tag${status.new_tags_added !== 1 ? 's' : ''} added to ${status.files_newly_tagged} file${status.files_newly_tagged !== 1 ? 's' : ''}</p>`
            : '';

        return `<div class="lp-phase">
            <progress value="${pct}" max="100"></progress>
            <p class="lp-progress-text">${status.processed_files.toLocaleString()} / ${status.total_files.toLocaleString()} files (${pct}%)</p>
            ${extras.length ? `<p class="lp-stats">${extras.join(' &middot; ')}</p>` : ''}
            ${tagLine}
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
        const extSummary = formatExtSummary(status.by_extension || {});
        const tagLine = (status.new_tags_added > 0)
            ? `<p class="lp-stats">${status.new_tags_added} tag${status.new_tags_added !== 1 ? 's' : ''} added to ${status.files_newly_tagged} file${status.files_newly_tagged !== 1 ? 's' : ''}</p>`
            : '';
        return `<div class="lp-phase">
            <p><strong>${status.total_files.toLocaleString()}</strong> files in library${extSummary}.</p>
            <p class="lp-stats">${status.processed_files.toLocaleString()} processed. No new files to process, press Scan to find new ones.</p>
            ${status.total_tags_assigned > 0 ? `<p class="lp-stats">${status.total_tags_assigned.toLocaleString()} tags assigned.</p>` : ''}
            ${status.failed_files > 0 ? `<p class="lp-stats">${status.failed_files.toLocaleString()} can't be read (failed).</p>` : ''}
            ${status.ocr_failed_pdfs > 0 ? `<p class="lp-stats">${status.ocr_failed_pdfs.toLocaleString()} PDF${status.ocr_failed_pdfs !== 1 ? 's' : ''} still unreadable after OCR, no extractable text.</p>` : ''}
            ${status.skipped_files > 0 ? `<p class="lp-stats">${status.skipped_files.toLocaleString()} skipped.</p>` : ''}
            ${tagLine}
        </div>`;
    }

    function renderExploringPhase(status) {
        const pct = status.explore_total > 0
            ? Math.round((status.explore_processed / status.explore_total) * 100) : 0;
        return `<div class="lp-phase">
            <progress value="${pct}" max="100"></progress>
            <p class="lp-progress-text">Files ${status.explore_processed} / ${status.explore_total}</p>
            ${status.explore_existing_tagged > 0 ? `<p class="lp-stats">${status.explore_existing_tagged} tags assigned to ${status.explore_existing_files} files so far</p>` : ''}
            <p style="font-size:0.82rem;color:var(--pico-muted-color);">Analyzing untagged files for new categories&hellip;</p>
            <div class="lp-actions">
                <button id="lp-explore-stop-btn" class="outline secondary">Stop Exploring</button>
            </div>
        </div>`;
    }

    function renderExploreStoppingPhase(status) {
        const pct = status.explore_total > 0
            ? Math.round((status.explore_processed / status.explore_total) * 100) : 0;
        return `<div class="lp-phase">
            <progress value="${pct}" max="100"></progress>
            <p class="lp-progress-text">Files ${status.explore_processed} / ${status.explore_total}</p>
            <p style="font-size:0.82rem;color:var(--pico-muted-color);">Stopping after current batch&hellip;</p>
            <div class="lp-actions">
                <button disabled class="outline secondary" aria-busy="true">Stopping&hellip;</button>
            </div>
        </div>`;
    }

    function renderOCRPhase(status) {
        const pct = status.ocr_total > 0
            ? Math.round((status.ocr_processed / status.ocr_total) * 100) : 0;
        const extras = [];
        if (status.ocr_success > 0) extras.push(`${status.ocr_success} succeeded`);
        if (status.ocr_failed > 0) extras.push(`${status.ocr_failed} failed`);

        return `<div class="lp-phase">
            <progress value="${pct}" max="100"></progress>
            <p class="lp-progress-text">${status.ocr_processed} / ${status.ocr_total} files (${pct}%)</p>
            ${extras.length ? `<p class="lp-stats">${extras.join(' &middot; ')}</p>` : ''}
            ${status.ocr_current_file ? `<p class="lp-stats" style="font-family:monospace;font-size:0.7rem;word-break:break-all;">${escapeHtml(status.ocr_current_file)}</p>` : ''}
            <p style="font-size:0.82rem;color:var(--pico-muted-color);">Making scanned PDFs searchable. Re-process files after OCR completes.</p>
            <div class="lp-actions">
                <button id="lp-ocr-stop-btn" class="outline secondary">Stop OCR</button>
            </div>
        </div>`;
    }

    function renderOCRStoppingPhase(status) {
        const pct = status.ocr_total > 0
            ? Math.round((status.ocr_processed / status.ocr_total) * 100) : 0;
        return `<div class="lp-phase">
            <progress value="${pct}" max="100"></progress>
            <p class="lp-progress-text">${status.ocr_processed} / ${status.ocr_total} files (${pct}%)</p>
            <p style="font-size:0.82rem;color:var(--pico-muted-color);">Stopping after current file&hellip;</p>
            <div class="lp-actions">
                <button disabled class="outline secondary" aria-busy="true">Stopping&hellip;</button>
            </div>
        </div>`;
    }

    function renderExploreCompletePhase(status) {
        const candidates = status.explore_candidates || [];
        const existingLine = status.explore_existing_tagged > 0
            ? `<p class="lp-stats">${status.explore_existing_tagged} existing tags assigned to ${status.explore_existing_files} files.</p>`
            : '';

        if (!candidates.length) {
            return `<div class="lp-phase">
                <p>Exploration complete. No new tag categories were discovered that meet the minimum file threshold.</p>
                ${existingLine}
                <div class="lp-actions">
                    <button id="lp-explore-dismiss" class="outline">Dismiss</button>
                </div>
            </div>`;
        }

        const cardsHtml = candidates.map((c, i) => `
            <div class="lp-explore-card">
                <div class="lp-explore-card-header">
                    <input type="checkbox" class="lp-explore-cb" data-index="${i}" checked>
                    <span class="lp-explore-swatch" style="background:${escapeHtml(c.color)}"></span>
                    <strong>${escapeHtml(c.display_name)}</strong>
                    <span class="lp-explore-count">${c.file_count} files</span>
                </div>
                <p class="lp-explore-desc">${escapeHtml(c.description)}</p>
            </div>
        `).join('');

        return `<div class="lp-phase">
            <p>Found <strong>${candidates.length}</strong> potential new categories:</p>
            ${existingLine}
            <div class="lp-explore-candidates">${cardsHtml}</div>
            <div class="lp-actions" style="margin-top:0.75rem;">
                <button id="lp-explore-accept">Accept Selected Tags</button>
                <button id="lp-explore-dismiss" class="outline secondary">Dismiss All</button>
            </div>
        </div>`;
    }

    function renderActionButtons(status) {
        const busy = window.chatIsStreaming?.();
        const processDisabled = (status.new_files === 0 || busy) ? ' disabled' : '';
        const scanDisabled = busy ? ' disabled' : '';
        const exploreDisabled = busy ? ' disabled' : '';
        const ocrCount = status.failed_pdfs || 0;
        const ocrDisabled = (ocrCount === 0 || busy) ? ' disabled' : '';
        return `<hr class="lp-divider">
        <div class="lp-phase">
            <div class="lp-actions">
                <button id="lp-action-scan" class="outline"${scanDisabled}>Scan Files</button>
                <button id="lp-action-process-new" class="outline"${processDisabled}>Process New Files</button>
                <button id="lp-action-ocr" class="outline"${ocrDisabled}>OCR Failed Files${ocrCount > 0 ? ' (' + ocrCount + ')' : ''}</button>
                <button id="lp-action-explore" class="outline"${exploreDisabled}>Explore New Tags</button>
            </div>
            <hr class="lp-divider" style="margin:0.5rem 0">
            ${renderBackupActions(status)}
        </div>`;
    }

    // Export + Import backup buttons. Used by renderActionButtons() in the
    // 'complete' phase, and separately in the empty/scanned phases so users
    // can restore a backup before any processing happens. Export is disabled
    // when there's nothing to export yet.
    function renderBackupActions(status) {
        const busy = window.chatIsStreaming?.();
        const importDisabled = busy ? ' disabled' : '';
        const exportDisabled = (busy || (status.processed_files || 0) === 0) ? ' disabled' : '';
        return `<div class="lp-actions">
            <button id="lp-action-export" class="outline"${exportDisabled}>Export Backup</button>
            <button id="lp-action-import" class="outline"${importDisabled}>Import Backup</button>
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
        document.getElementById('lp-stop-btn')
            ?.addEventListener('click', handleStop);

        // Action buttons
        document.getElementById('lp-action-scan')
            ?.addEventListener('click', handleActionScan);
        document.getElementById('lp-action-process-new')
            ?.addEventListener('click', handleProcessNew);
        document.getElementById('lp-action-reprocess')
            ?.addEventListener('click', handleReprocessAll);

        // OCR buttons
        document.getElementById('lp-action-ocr')
            ?.addEventListener('click', handleStartOCR);
        document.getElementById('lp-ocr-stop-btn')
            ?.addEventListener('click', handleStopOCR);

        // Explore buttons
        document.getElementById('lp-action-explore')
            ?.addEventListener('click', handleStartExploring);
        document.getElementById('lp-explore-stop-btn')
            ?.addEventListener('click', handleStopExploring);
        document.getElementById('lp-explore-accept')
            ?.addEventListener('click', handleAcceptExploreTags);
        document.getElementById('lp-explore-dismiss')
            ?.addEventListener('click', handleDismissExplore);

        // Backup buttons
        document.getElementById('lp-action-export')
            ?.addEventListener('click', handleStartExport);
        document.getElementById('lp-action-import')
            ?.addEventListener('click', handleImportClick);
        document.getElementById('lp-backup-stop-btn')
            ?.addEventListener('click', handleStopBackup);
    }

    // --- Action Handlers ---

    async function handleScan() {
        if (window.chatIsStreaming?.()) return;
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
        if (!(await window.requireLlmServers())) return;
        const restore = setBtnBusy(document.getElementById('lp-process-btn'), null);
        const result = await apiPost('/api/processing/start', {});
        if (result.error) {
            alert(result.error);
            restore();
            return;
        }
        lastStatusJSON = '';
        await checkStatus();
        window.syncChannel.postMessage({ type: 'processing-started', payload: {} });
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
        if (window.chatIsStreaming?.()) return;
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
        if (!(await window.requireLlmServers())) return;
        const restore = setBtnBusy(document.getElementById('lp-action-process-new'), null);
        const result = await apiPost('/api/processing/start', {});
        if (result.error) {
            alert(result.error);
            restore();
            return;
        }
        lastStatusJSON = '';
        await checkStatus();
        window.syncChannel.postMessage({ type: 'processing-started', payload: {} });
    }

    async function handleReprocessAll() {
        if (window.chatIsStreaming?.()) return;
        const dialog = document.getElementById('reprocess-confirm-dialog');
        dialog.showModal();
    }

    async function confirmReprocessAll() {
        const dialog = document.getElementById('reprocess-confirm-dialog');
        dialog.close();
        if (!(await window.requireLlmServers())) return;
        const restore = setBtnBusy(document.getElementById('lp-action-reprocess'), null);
        const result = await apiPost('/api/processing/start', { reprocess: true });
        if (result.error) {
            alert(result.error);
            restore();
            return;
        }
        lastStatusJSON = '';
        await checkStatus();
        window.syncChannel.postMessage({ type: 'processing-started', payload: {} });
    }

    // --- OCR Handlers ---

    async function handleStartOCR() {
        if (window.chatIsStreaming?.()) return;
        if (!(await window.requireLlmServers())) return;
        const restore = setBtnBusy(document.getElementById('lp-action-ocr'), null);
        const result = await apiPost('/api/ocr/start');
        if (result.error) {
            alert(result.error);
            restore();
            return;
        }
        lastStatusJSON = '';
        await checkStatus();
        window.syncChannel.postMessage({ type: 'processing-started', payload: {} });
    }

    async function handleStopOCR() {
        const btn = document.getElementById('lp-ocr-stop-btn');
        if (btn) {
            btn.disabled = true;
            btn.textContent = 'Stopping\u2026';
        }
        await apiPost('/api/ocr/stop');
        lastStatusJSON = '';
        await checkStatus();
    }

    // --- Explore Handlers ---

    async function handleStartExploring() {
        if (window.chatIsStreaming?.()) return;
        if (!(await window.requireLlmServers())) return;
        const restore = setBtnBusy(document.getElementById('lp-action-explore'), null);
        const result = await apiPost('/api/explore/start');
        if (result.error) {
            alert(result.error);
            restore();
            return;
        }
        lastStatusJSON = '';
        await checkStatus();
        window.syncChannel.postMessage({ type: 'processing-started', payload: {} });
    }

    async function handleStopExploring() {
        const btn = document.getElementById('lp-explore-stop-btn');
        if (btn) {
            btn.disabled = true;
            btn.textContent = 'Stopping\u2026';
        }
        await apiPost('/api/explore/stop');
        lastStatusJSON = '';
        await checkStatus();
    }

    async function handleAcceptExploreTags() {
        const checkboxes = document.querySelectorAll('.lp-explore-cb:checked');
        const lastStatus = JSON.parse(lastStatusJSON);
        const candidates = lastStatus.explore_candidates || [];

        const selected = [];
        checkboxes.forEach(cb => {
            const idx = parseInt(cb.dataset.index);
            if (candidates[idx]) {
                selected.push({
                    name: candidates[idx].name,
                    display_name: candidates[idx].display_name,
                    description: candidates[idx].description,
                    color: candidates[idx].color,
                    filenames: candidates[idx].filenames || [],
                });
            }
        });

        if (!selected.length) {
            alert('No tags selected.');
            return;
        }

        const btn = document.getElementById('lp-explore-accept');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');

        const result = await apiPost('/api/explore/accept', { tags: selected });
        if (result.error) {
            alert(result.error);
            btn.disabled = false;
            btn.setAttribute('aria-busy', 'false');
            return;
        }

        lastStatusJSON = '';
        await checkStatus();
        window.syncChannel.postMessage({ type: 'tags-changed', payload: {} });
    }

    async function handleDismissExplore() {
        await apiPost('/api/explore/dismiss');
        lastStatusJSON = '';
        await checkStatus();
    }

    // --- Backup Handlers ---

    let backupPollTimer = null;

    async function handleStartExport() {
        if (window.chatIsStreaming?.()) return;
        const btn = document.getElementById('lp-action-export');
        if (btn) {
            btn.disabled = true;
            btn.setAttribute('aria-busy', 'true');
        }
        try {
            await apiPost('/api/backup/export', { include_rag: true });
            pollBackupProgress('export');
        } catch (e) {
            alert('Export failed: ' + (e.message || e));
            if (btn) {
                btn.disabled = false;
                btn.setAttribute('aria-busy', 'false');
            }
        }
    }

    function handleImportClick() {
        if (window.chatIsStreaming?.()) return;
        // Open file picker
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = '.georag';
        input.onchange = async () => {
            const file = input.files[0];
            if (!file) return;

            // Validate first
            const btn = document.getElementById('lp-action-import');
            if (btn) {
                btn.disabled = true;
                btn.setAttribute('aria-busy', 'true');
            }
            try {
                const form = new FormData();
                form.append('file', file);
                const resp = await fetch('/api/backup/import/validate', { method: 'POST', body: form });
                const result = await resp.json();

                if (!result.valid) {
                    alert('Invalid backup: ' + result.error);
                    if (btn) { btn.disabled = false; btn.setAttribute('aria-busy', 'false'); }
                    return;
                }

                // Show confirmation dialog
                showImportConfirmDialog(file, result);
            } catch (e) {
                alert('Validation failed: ' + (e.message || e));
            }
            if (btn) { btn.disabled = false; btn.setAttribute('aria-busy', 'false'); }
        };
        input.click();
    }

    function showImportConfirmDialog(file, validation) {
        const dlg = document.getElementById('import-confirm-dialog');
        if (!dlg) return;
        const m = validation.manifest || {};
        const s = m.stats || {};
        const warnings = validation.warnings || [];
        const includes = m.includes || {};

        let body = `<p>This will <strong>replace all existing data</strong> with the contents of this backup.</p>`;
        body += `<ul>`;
        if (s.files) body += `<li>${s.files.toLocaleString()} files</li>`;
        if (s.tags) body += `<li>${s.tags} tags</li>`;
        if (s.file_tags) body += `<li>${s.file_tags.toLocaleString()} tag assignments</li>`;
        if (s.total_chunks) body += `<li>${s.total_chunks.toLocaleString()} vector chunks</li>`;
        body += `</ul>`;
        body += `<p style="font-size:0.8rem;color:var(--pico-muted-color)">From: ${m.source_platform || 'unknown'} | Model: ${m.embedding_model || 'unknown'}</p>`;
        if (warnings.length) {
            body += `<p style="color:var(--pico-del-color);font-size:0.85rem">${warnings.join('<br>')}</p>`;
        }
        body += `<p>A safety backup of your current database will be created before importing.</p>`;

        dlg.querySelector('.import-dialog-body').innerHTML = body;
        dlg._importFile = file;
        dlg.showModal();
    }

    async function confirmImport() {
        const dlg = document.getElementById('import-confirm-dialog');
        dlg.close();
        const file = dlg._importFile;
        if (!file) return;

        const btn = document.getElementById('lp-action-import');
        if (btn) {
            btn.disabled = true;
            btn.setAttribute('aria-busy', 'true');
        }
        try {
            const form = new FormData();
            form.append('file', file);
            const resp = await fetch('/api/backup/import', { method: 'POST', body: form });
            const result = await resp.json();
            if (result.error || result.detail) {
                alert('Import failed: ' + (result.error || result.detail));
                if (btn) { btn.disabled = false; btn.setAttribute('aria-busy', 'false'); }
                return;
            }
            pollBackupProgress('import');
        } catch (e) {
            alert('Import failed: ' + (e.message || e));
            if (btn) { btn.disabled = false; btn.setAttribute('aria-busy', 'false'); }
        }
    }

    function pollBackupProgress(type) {
        if (backupPollTimer) clearInterval(backupPollTimer);

        // Replace panel content with progress view
        const url = type === 'export' ? '/api/backup/export/status' : '/api/backup/import/status';
        const label = type === 'export' ? 'Exporting' : 'Importing';

        backupPollTimer = setInterval(async () => {
            try {
                const status = await apiGet(url);

                // Build progress HTML
                let progressHtml = `<div class="lp-header">
                    <h3>Library</h3>
                    <button class="lp-close" id="lp-close-btn" aria-label="Close">&times;</button>
                </div>`;
                progressHtml += `<div class="lp-phase">`;

                if (status.phase === 'done') {
                    clearInterval(backupPollTimer);
                    backupPollTimer = null;

                    if (type === 'export') {
                        progressHtml += `<p>${label} complete. ${status.chunks_exported?.toLocaleString() || 0} chunks exported.</p>`;
                        progressHtml += `<div class="lp-actions">
                            <button id="lp-backup-download" class="outline">Download Backup</button>
                        </div>`;
                    } else {
                        progressHtml += `<p>Import complete. ${status.chunks_imported?.toLocaleString() || 0} chunks restored.</p>`;
                        if (status.warnings?.length) {
                            progressHtml += `<p style="color:var(--pico-del-color);font-size:0.85rem">${status.warnings.join('<br>')}</p>`;
                        }
                        progressHtml += `<div class="lp-actions">
                            <button id="lp-backup-reload">Reload Page</button>
                        </div>`;
                    }
                } else if (status.phase === 'error') {
                    clearInterval(backupPollTimer);
                    backupPollTimer = null;
                    progressHtml += `<p style="color:var(--pico-del-color)">${label} failed: ${status.error || 'Unknown error'}</p>`;
                    progressHtml += `<div class="lp-actions">
                        <button id="lp-backup-dismiss" class="outline">Dismiss</button>
                    </div>`;
                } else {
                    const pct = status.collections_total > 0
                        ? Math.round((status.collections_done / status.collections_total) * 100) : 0;
                    const phaseLabel = {
                        sqlite: 'Saving database...',
                        vectors: `${label} vectors...`,
                        packaging: 'Packaging archive...',
                        validating: 'Validating archive...',
                        clearing: 'Clearing existing data...',
                        finalizing: 'Finalizing...',
                    }[status.phase] || `${label}...`;

                    progressHtml += `<progress value="${pct}" max="100"></progress>`;
                    progressHtml += `<p class="lp-progress-text">${phaseLabel}</p>`;
                    if (status.current_collection) {
                        progressHtml += `<p class="lp-stats">${status.current_collection} (${status.collections_done}/${status.collections_total})</p>`;
                    }
                    const chunkLabel = type === 'export' ? status.chunks_exported : status.chunks_imported;
                    if (chunkLabel) {
                        progressHtml += `<p class="lp-stats">${chunkLabel.toLocaleString()} chunks</p>`;
                    }
                    progressHtml += `<div class="lp-actions">
                        <button id="lp-backup-stop-btn" class="outline secondary">Stop</button>
                    </div>`;
                }

                progressHtml += `</div>`;
                contentEl.innerHTML = progressHtml;

                // Bind progress-specific events
                document.getElementById('lp-close-btn')?.addEventListener('click', closePanel);
                document.getElementById('lp-backup-stop-btn')?.addEventListener('click', handleStopBackup);
                document.getElementById('lp-backup-download')?.addEventListener('click', () => {
                    window.location.href = '/api/backup/export/download';
                    // Return to normal panel after a moment
                    setTimeout(() => { lastStatusJSON = ''; checkStatus(); }, 1500);
                });
                document.getElementById('lp-backup-reload')?.addEventListener('click', () => {
                    window.location.reload();
                });
                document.getElementById('lp-backup-dismiss')?.addEventListener('click', () => {
                    lastStatusJSON = '';
                    checkStatus();
                });
            } catch {
                // API not available, keep polling
            }
        }, POLL_MS);
    }

    async function handleStopBackup() {
        try {
            await apiPost('/api/backup/export/stop');
            await apiPost('/api/backup/import/stop');
        } catch {}
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

    return { init, openPanel, closePanel, toggle, checkStatus, confirmReprocessAll, confirmImport };
})();

document.addEventListener('DOMContentLoaded', () => {
    LibraryPanel.init();

    // Reprocess confirmation dialog
    const reprocessDialog = document.getElementById('reprocess-confirm-dialog');
    if (reprocessDialog) {
        reprocessDialog.querySelector('.dialog-cancel')
            ?.addEventListener('click', () => reprocessDialog.close());
        reprocessDialog.querySelector('.dialog-confirm')
            ?.addEventListener('click', () => LibraryPanel.confirmReprocessAll());
    }

    // Import confirmation dialog
    const importDialog = document.getElementById('import-confirm-dialog');
    if (importDialog) {
        importDialog.querySelector('.dialog-cancel')
            ?.addEventListener('click', () => importDialog.close());
        importDialog.querySelector('.dialog-confirm')
            ?.addEventListener('click', () => LibraryPanel.confirmImport());
    }
});
