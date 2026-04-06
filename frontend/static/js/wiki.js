// Wiki page logic — page management, ingest, query, lint, library sync

(function () {
    let wikiInitialized = false;
    let currentPageSlug = null;
    let wikiPages = [];
    let selectedCategory = '';
    let isQuerying = false;
    let queryAnswer = '';
    let queryPagesUsed = [];
    let allTags = [];
    let selectedIngestTags = new Set();
    let ingestPollTimer = null;
    let readinessPollTimer = null;
    let lastSyncStats = null;
    let lastIngestTagNames = [];
    let lastIngestFileIds = [];

    // ── Initialization ───────────────────────────────────────────────────

    document.addEventListener('spa:pageshow', (e) => {
        if (e.detail.page !== 'wiki') return;
        if (!wikiInitialized) {
            initWiki();
            wikiInitialized = true;
        }
        loadPages();
        updateWikiReadiness();
        startReadinessPolling();
    });

    // Re-clicking the wiki nav link while already on wiki returns to welcome screen
    document.addEventListener('click', (e) => {
        const link = e.target.closest('nav a[href="/wiki"]');
        if (!link) return;
        if (document.body.dataset.activePage !== 'wiki') return;
        currentPageSlug = null;
        document.getElementById('wiki-page-view').style.display = 'none';
        document.getElementById('wiki-query-response').style.display = 'none';
        document.getElementById('wiki-welcome').style.display = '';
        updateWikiReadiness();
    });

    // Stop readiness polling when navigating away from wiki
    document.addEventListener('spa:pageshow', (e) => {
        if (e.detail.page !== 'wiki') {
            stopReadinessPolling();
        }
    });

    // Listen for cross-tab library events
    if (window.syncChannel) {
        window.syncChannel.addEventListener('message', (e) => {
            if (e.data.type === 'processing-started' || e.data.type === 'tags-changed') {
                updateWikiReadiness();
            }
        });
    }

    function initWiki() {
        // Category filters
        document.getElementById('wiki-categories').addEventListener('click', (e) => {
            const btn = e.target.closest('.wiki-cat-btn');
            if (!btn) return;
            document.querySelectorAll('.wiki-cat-btn').forEach(b => b.classList.remove('selected'));
            btn.classList.add('selected');
            selectedCategory = btn.dataset.category;
            renderPageList();
        });

        // Search
        const searchInput = document.getElementById('wiki-search-input');
        let searchTimeout;
        searchInput.addEventListener('input', () => {
            clearTimeout(searchTimeout);
            searchTimeout = setTimeout(() => {
                const q = searchInput.value.trim();
                if (q.length >= 2) {
                    searchPages(q);
                } else {
                    renderPageList();
                }
            }, 300);
        });

        // Actions
        document.getElementById('wiki-ingest-btn').addEventListener('click', openIngestDialog);
        document.getElementById('wiki-lint-btn').addEventListener('click', runLint);
        document.getElementById('wiki-new-btn').addEventListener('click', openNewPageDialog);
        document.getElementById('wiki-delete-btn').addEventListener('click', confirmDeletePage);

        // Query form
        document.getElementById('wiki-query-form').addEventListener('submit', (e) => {
            e.preventDefault();
            submitQuery();
        });
        document.getElementById('wiki-query-input').addEventListener('keydown', (e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                submitQuery();
            }
        });
        document.getElementById('wiki-query-close-btn').addEventListener('click', closeQueryResponse);
        document.getElementById('wiki-query-save-btn').addEventListener('click', saveQueryAsPage);

        // Initialize Wiki button (ready state)
        document.getElementById('wiki-init-btn')?.addEventListener('click', initializeWiki);

        // Update Wiki button (populated state)
        document.getElementById('wiki-update-btn')?.addEventListener('click', () => {
            initializeWiki();
        });

        // Stop/Resume/Pending buttons
        document.getElementById('wiki-init-stop-btn')?.addEventListener('click', stopIngest);
        document.getElementById('wiki-ingest-stop-btn')?.addEventListener('click', stopIngest);
        document.getElementById('wiki-resume-btn')?.addEventListener('click', resumeIngest);
        document.getElementById('wiki-process-pending-btn')?.addEventListener('click', processPendingFiles);

        // Ingest dialog
        document.querySelectorAll('.close-wiki-ingest').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('wiki-ingest-dialog').close());
        });
        document.getElementById('wiki-ingest-start-btn').addEventListener('click', startIngest);

        // Lint dialog
        document.querySelectorAll('.close-wiki-lint').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('wiki-lint-dialog').close());
        });
        document.getElementById('wiki-lint-apply-btn').addEventListener('click', applyLintFixes);
        document.getElementById('wiki-lint-stop-btn').addEventListener('click', stopLintFix);

        // New page dialog
        document.querySelectorAll('.close-wiki-new-page').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('wiki-new-page-dialog').close());
        });
        document.getElementById('wiki-new-create-btn').addEventListener('click', createNewPage);

        // Delete dialog
        document.querySelector('#wiki-delete-dialog .dialog-cancel').addEventListener('click', () => {
            document.getElementById('wiki-delete-dialog').close();
        });

        // Reset Wiki dialog
        document.getElementById('wiki-reset-btn')?.addEventListener('click', () => {
            document.getElementById('wiki-reset-dialog').showModal();
        });
        document.querySelector('.wiki-reset-cancel-btn')?.addEventListener('click', () => {
            document.getElementById('wiki-reset-dialog').close();
        });
        document.querySelector('.wiki-reset-confirm-btn')?.addEventListener('click', resetWiki);
        document.querySelector('.wiki-delete-confirm-btn').addEventListener('click', deletePage);

        // Log dialog
        document.querySelectorAll('.close-wiki-log').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('wiki-log-dialog').close());
        });

        // Textarea input → toggle Ask button enabled/disabled based on text
        document.getElementById('wiki-query-input').addEventListener('input', () => {
            updateQueryInputState();
        });

        // Set initial query input state
        updateQueryInputState();
    }

    // ── Query Input State (LLM busy / wiki not ready) ───────────────────

    function updateQueryInputState() {
        const input = document.getElementById('wiki-query-input');
        const btn = document.getElementById('wiki-query-btn');
        if (!input || !btn) return;

        // Don't override state while actively querying
        if (isQuerying) return;

        const llmBusy = window.chatIsStreaming?.() || window.libraryIsProcessing;
        const wikiNotReady = lastSyncStats && (
            lastSyncStats.wiki_total_pages === 0 && !lastSyncStats.wiki_ever_ingested
        );
        const wikiIngesting = lastSyncStats?.wiki_ingest_running;

        if (llmBusy) {
            input.disabled = true;
            btn.disabled = true;
            input.placeholder = 'LLM is busy with another task...';
        } else if (wikiIngesting) {
            input.disabled = true;
            btn.disabled = true;
            input.placeholder = 'Wiki is being built...';
        } else if (wikiNotReady) {
            input.disabled = true;
            btn.disabled = true;
            input.placeholder = 'Initialize the wiki first to ask questions...';
        } else {
            input.disabled = false;
            input.placeholder = 'Ask the wiki a question...';
            btn.disabled = !input.value.trim();
        }
    }

    // Expose for cross-component state updates (panel.js, chat.js)
    window.updateWikiQueryState = function() {
        updateQueryInputState();
        // Also update action buttons (Rebuild/Health Check) since they are LLM-dependent
        if (lastSyncStats) updateActionButtonStates(lastSyncStats);
    };

    // ── Wiki Readiness / State Machine ───────────────────────────────────

    function startReadinessPolling() {
        stopReadinessPolling();
        readinessPollTimer = setInterval(updateWikiReadiness, 3000);
    }

    function stopReadinessPolling() {
        if (readinessPollTimer) {
            clearInterval(readinessPollTimer);
            readinessPollTimer = null;
        }
    }

    async function updateWikiReadiness() {
        try {
            const stats = await apiGet('/api/wiki/stats');
            lastSyncStats = stats;
            applyWikiState(stats);
            updateIngestButtonBadge(stats);
            updateActionButtonStates(stats);
            updateQueryInputState();
        } catch (e) {
            // API not available yet
        }
    }

    function applyWikiState(stats) {
        // Hide all states first
        document.querySelectorAll('#wiki-welcome .wiki-state').forEach(el => {
            el.style.display = 'none';
        });

        // Only update welcome states if welcome is visible
        // (don't override page view or query response)
        const welcome = document.getElementById('wiki-welcome');
        if (welcome.style.display === 'none') return;

        if (stats.wiki_ingest_running) {
            // Ingest is running — show progress
            document.getElementById('wiki-state-ingesting').style.display = '';
            pollInitProgress();
        } else if (stats.wiki_ingest_stopped) {
            // Ingest was stopped — show paused state with resume
            document.getElementById('wiki-state-stopped').style.display = '';
            fetchAndShowStoppedProgress();
        } else if (stats.library_total_files === 0) {
            // No documents at all
            document.getElementById('wiki-state-empty-library').style.display = '';
        } else if (stats.library_is_processing) {
            // Library is mid-processing
            document.getElementById('wiki-state-processing').style.display = '';
            updateProcessingProgress(stats);
        } else if (stats.wiki_total_pages === 0 && !stats.wiki_ever_ingested) {
            // Library done, wiki never initialized
            document.getElementById('wiki-state-ready').style.display = '';
            document.getElementById('wiki-ready-count').textContent =
                stats.library_processed_files.toLocaleString();
        } else {
            // Wiki has pages — show populated state
            document.getElementById('wiki-state-populated').style.display = '';
            // Show update banner if new files since last ingest
            const banner = document.getElementById('wiki-update-banner');
            if (stats.files_since_last_ingest > 0) {
                banner.style.display = '';
                document.getElementById('wiki-update-count').textContent =
                    stats.files_since_last_ingest.toLocaleString();
            } else {
                banner.style.display = 'none';
            }
            // Show coverage stats if there are pending files
            const coverageDiv = document.getElementById('wiki-coverage-stats');
            if (coverageDiv) {
                if (stats.wiki_pending_files > 0) {
                    coverageDiv.style.display = '';
                    document.getElementById('wiki-covered-count').textContent =
                        (stats.wiki_covered_files ?? 0).toLocaleString();
                    document.getElementById('wiki-pending-count').textContent =
                        stats.wiki_pending_files.toLocaleString();
                } else {
                    coverageDiv.style.display = 'none';
                }
            }
        }
    }

    async function fetchAndShowStoppedProgress() {
        try {
            const status = await apiGet('/api/wiki/ingest/status');
            const bar = document.getElementById('wiki-stopped-progress-bar');
            const text = document.getElementById('wiki-stopped-progress-text');
            if (bar && text && status.total_sources > 0) {
                const pct = Math.round((status.processed_sources / status.total_sources) * 100);
                bar.value = pct;
                text.textContent = `${status.processed_sources}/${status.total_sources} sources processed. ` +
                    `${status.pages_created} created, ${status.pages_updated} updated. Paused.`;
            }
        } catch (e) { /* ignore */ }
    }

    function updateProcessingProgress(stats) {
        const bar = document.getElementById('wiki-lib-progress');
        const text = document.getElementById('wiki-lib-progress-text');
        if (stats.library_total_files > 0) {
            const pct = Math.round((stats.library_processed_files / stats.library_total_files) * 100);
            bar.value = pct;
            text.textContent = `${stats.library_processed_files.toLocaleString()} / ${stats.library_total_files.toLocaleString()} files processed (${pct}%)`;
        }
    }

    function updateIngestButtonBadge(stats) {
        const ingestBtn = document.getElementById('wiki-ingest-btn');
        if (stats.files_since_last_ingest > 0 && stats.wiki_ever_ingested) {
            ingestBtn.innerHTML = `Rebuild for Tag <span class="wiki-update-badge">${stats.files_since_last_ingest}</span>`;
            ingestBtn.title = `${stats.files_since_last_ingest} new files since last ingest`;
        } else {
            ingestBtn.textContent = 'Rebuild for Tag';
            ingestBtn.title = 'Rebuild wiki from tagged documents';
        }
    }

    function updateActionButtonStates(stats) {
        const ingestBtn = document.getElementById('wiki-ingest-btn');
        const lintBtn = document.getElementById('wiki-lint-btn');
        const newBtn = document.getElementById('wiki-new-btn');
        const resetBtn = document.getElementById('wiki-reset-btn');
        const pendingBtn = document.getElementById('wiki-process-pending-btn');

        const llmBusy = window.chatIsStreaming?.() || stats.library_is_processing || stats.wiki_ingest_running;

        if (llmBusy) {
            ingestBtn.disabled = true;
            lintBtn.disabled = true;
            newBtn.disabled = true;
            resetBtn.disabled = true;
            if (pendingBtn) pendingBtn.disabled = true;
            if (window.chatIsStreaming?.()) {
                ingestBtn.title = 'Chat is streaming — wait for it to finish';
                lintBtn.title = 'Chat is streaming — wait for it to finish';
                newBtn.title = 'Chat is streaming — wait for it to finish';
                resetBtn.title = 'Chat is streaming — wait for it to finish';
            } else if (stats.library_is_processing) {
                ingestBtn.title = 'Library is still processing documents...';
                lintBtn.title = 'Library is still processing documents...';
                newBtn.title = 'Library is still processing documents...';
                resetBtn.title = 'Library is still processing documents...';
            } else if (stats.wiki_ingest_running) {
                ingestBtn.title = 'Wiki rebuild is already running...';
                lintBtn.title = 'Wiki rebuild is already running...';
                newBtn.title = 'Wiki rebuild is already running...';
                resetBtn.title = 'Wiki rebuild is already running...';
            }
        } else {
            ingestBtn.disabled = false;
            lintBtn.disabled = false;
            newBtn.disabled = false;
            resetBtn.disabled = false;
            if (pendingBtn) pendingBtn.disabled = false;
            ingestBtn.title = 'Rebuild wiki from tagged documents';
            lintBtn.title = 'Run a health check on the wiki';
            newBtn.title = 'Create a manual wiki page';
            resetBtn.title = 'Delete all wiki pages and reset';
        }
    }

    // ── Initialize Wiki (one-click ingest all) ───────────────────────────

    async function initializeWiki() {
        if (!await window.requireLmStudio()) return;

        // Save params for resume
        lastIngestTagNames = [];
        lastIngestFileIds = [];

        // Show ingesting state
        document.querySelectorAll('#wiki-welcome .wiki-state').forEach(el => {
            el.style.display = 'none';
        });
        document.getElementById('wiki-state-ingesting').style.display = '';
        document.getElementById('wiki-init-progress-text').textContent = 'Starting wiki initialization...';
        document.getElementById('wiki-init-progress-bar').value = 0;

        try {
            await apiPost('/api/wiki/ingest', { tag_names: [], file_ids: [] });
            pollInitProgress();
        } catch (e) {
            document.getElementById('wiki-init-progress-text').textContent =
                'Error: ' + (e.message || 'Failed to start ingest');
        }
    }

    async function stopIngest() {
        try {
            await apiPost('/api/wiki/ingest/stop');
            // UI will update via poll detecting phase="stopping" then "stopped"
        } catch (e) {
            console.error('Failed to stop ingest:', e);
        }
    }

    async function resumeIngest() {
        if (!await window.requireLmStudio()) return;

        // Show ingesting state
        document.querySelectorAll('#wiki-welcome .wiki-state').forEach(el => {
            el.style.display = 'none';
        });
        document.getElementById('wiki-state-ingesting').style.display = '';
        document.getElementById('wiki-init-progress-text').textContent = 'Resuming...';

        try {
            await apiPost('/api/wiki/ingest', {
                tag_names: lastIngestTagNames,
                file_ids: lastIngestFileIds,
            });
            pollInitProgress();
        } catch (e) {
            document.getElementById('wiki-init-progress-text').textContent =
                'Error: ' + (e.message || 'Failed to resume ingest');
        }
    }

    async function processPendingFiles() {
        if (!await window.requireLmStudio()) return;

        document.querySelectorAll('#wiki-welcome .wiki-state').forEach(el => {
            el.style.display = 'none';
        });
        document.getElementById('wiki-state-ingesting').style.display = '';
        document.getElementById('wiki-init-progress-text').textContent = 'Starting ingest for pending files...';
        document.getElementById('wiki-init-progress-bar').value = 0;

        try {
            await apiPost('/api/wiki/ingest/pending');
            pollInitProgress();
        } catch (e) {
            document.getElementById('wiki-init-progress-text').textContent =
                'Error: ' + (e.message || 'Failed to start ingest');
        }
    }

    async function resetWiki() {
        try {
            await apiDelete('/api/wiki/reset');
            document.getElementById('wiki-reset-dialog').close();
            currentPageSlug = null;
            document.getElementById('wiki-page-view').style.display = 'none';
            document.getElementById('wiki-welcome').style.display = '';
            loadPages();
            updateWikiReadiness();
        } catch (e) {
            alert('Failed to reset wiki: ' + (e.message || 'Unknown error'));
        }
    }

    function pollInitProgress() {
        if (ingestPollTimer) clearInterval(ingestPollTimer);
        ingestPollTimer = setInterval(async () => {
            try {
                const status = await apiGet('/api/wiki/ingest/status');
                const progressText = status.total_sources > 0
                    ? `${status.processed_sources}/${status.total_sources} sources processed. ` +
                      `${status.pages_created} created, ${status.pages_updated} updated.` +
                      (status.current_source ? ` Current: ${status.current_source}` : '')
                    : 'Starting...';
                const pct = status.total_sources > 0
                    ? Math.round((status.processed_sources / status.total_sources) * 100) : 0;

                // Update inline progress in welcome area
                const bar = document.getElementById('wiki-init-progress-bar');
                const text = document.getElementById('wiki-init-progress-text');
                if (bar && text) {
                    bar.value = pct;
                    text.textContent = progressText;
                }

                // Update dialog progress if open
                const dlgBar = document.getElementById('wiki-ingest-progress-bar');
                const dlgText = document.getElementById('wiki-ingest-progress-text');
                if (dlgBar && dlgText) {
                    dlgBar.value = pct;
                    dlgText.textContent = progressText;
                }

                // Handle stopping phase — disable stop buttons, show "Stopping..."
                const inlineStopBtn = document.getElementById('wiki-init-stop-btn');
                const dlgStopBtn = document.getElementById('wiki-ingest-stop-btn');
                if (status.phase === 'stopping') {
                    if (inlineStopBtn) {
                        inlineStopBtn.disabled = true;
                        inlineStopBtn.setAttribute('aria-busy', 'true');
                        inlineStopBtn.textContent = 'Stopping\u2026';
                    }
                    if (dlgStopBtn) {
                        dlgStopBtn.disabled = true;
                        dlgStopBtn.setAttribute('aria-busy', 'true');
                        dlgStopBtn.textContent = 'Stopping\u2026';
                    }
                } else if (status.phase === 'ingesting') {
                    if (inlineStopBtn) {
                        inlineStopBtn.disabled = false;
                        inlineStopBtn.removeAttribute('aria-busy');
                        inlineStopBtn.textContent = 'Stop';
                    }
                }

                if (!status.is_running) {
                    clearInterval(ingestPollTimer);
                    ingestPollTimer = null;

                    if (status.was_stopped) {
                        // Show stopped/resume state inline
                        document.querySelectorAll('#wiki-welcome .wiki-state').forEach(el => {
                            el.style.display = 'none';
                        });
                        const welcome = document.getElementById('wiki-welcome');
                        if (welcome) welcome.style.display = '';
                        document.getElementById('wiki-state-stopped').style.display = '';

                        const stoppedBar = document.getElementById('wiki-stopped-progress-bar');
                        const stoppedText = document.getElementById('wiki-stopped-progress-text');
                        if (stoppedBar) stoppedBar.value = pct;
                        if (stoppedText) {
                            stoppedText.textContent = `${status.processed_sources}/${status.total_sources} sources processed. ` +
                                `${status.pages_created} created, ${status.pages_updated} updated. Paused.`;
                        }

                        // Update dialog if open
                        if (dlgStopBtn) dlgStopBtn.style.display = 'none';
                        const dlgStartBtn = document.getElementById('wiki-ingest-start-btn');
                        if (dlgStartBtn) {
                            dlgStartBtn.style.display = '';
                            dlgStartBtn.disabled = false;
                            dlgStartBtn.setAttribute('aria-busy', 'false');
                            dlgStartBtn.textContent = 'Resume Rebuild';
                        }
                    } else {
                        // Normal completion
                        const dlgStartBtn = document.getElementById('wiki-ingest-start-btn');
                        if (dlgStartBtn) {
                            dlgStartBtn.style.display = '';
                            dlgStartBtn.disabled = false;
                            dlgStartBtn.setAttribute('aria-busy', 'false');
                            dlgStartBtn.textContent = 'Done!';
                            if (dlgText) {
                                dlgText.textContent = `Complete. ${status.pages_created} created, ${status.pages_updated} updated.`;
                                if (status.errors && status.errors.length > 0) {
                                    dlgText.textContent += ` ${status.errors.length} errors.`;
                                }
                            }
                            setTimeout(() => { dlgStartBtn.textContent = 'Start Rebuild'; }, 2000);
                        }
                        if (dlgStopBtn) dlgStopBtn.style.display = 'none';
                    }

                    loadPages();
                    updateWikiReadiness();
                }
            } catch (e) {
                console.error('Ingest poll error:', e);
            }
        }, 2000);
    }

    // ── Page List ────────────────────────────────────────────────────────

    async function loadPages() {
        try {
            const url = selectedCategory
                ? `/api/wiki/pages?category=${encodeURIComponent(selectedCategory)}`
                : '/api/wiki/pages';
            wikiPages = await apiGet(url);
            renderPageList();
            updateStats();
        } catch (e) {
            console.error('Failed to load wiki pages:', e);
        }
    }

    function renderPageList() {
        const container = document.getElementById('wiki-page-list');
        const filtered = selectedCategory
            ? wikiPages.filter(p => p.category === selectedCategory)
            : wikiPages;

        if (filtered.length === 0) {
            container.innerHTML = '<p class="secondary" style="padding:1rem;font-size:0.85rem;">No pages in this category.</p>';
            return;
        }

        container.innerHTML = filtered.map(p => `
            <div class="wiki-page-item ${p.slug === currentPageSlug ? 'active' : ''}"
                 data-slug="${escapeHtml(p.slug)}">
                <div class="wiki-page-item-title">${escapeHtml(p.title)}</div>
                <div class="wiki-page-item-meta">
                    <span class="wiki-page-item-cat">${escapeHtml(p.category)}</span>
                    ${p.summary ? `<span class="wiki-page-item-summary">${escapeHtml(p.summary).substring(0, 80)}</span>` : ''}
                </div>
            </div>
        `).join('');

        // Click handlers
        container.querySelectorAll('.wiki-page-item').forEach(el => {
            el.addEventListener('click', () => {
                const slug = el.dataset.slug;
                loadPage(slug);
            });
        });
    }

    async function updateStats() {
        try {
            const stats = await apiGet('/api/wiki/stats');
            lastSyncStats = stats;
            const el = document.getElementById('wiki-stats');
            el.innerHTML = `<span class="secondary">${stats.total_pages} pages</span>`;
        } catch (e) { /* ignore */ }
    }

    // ── Page Viewer ──────────────────────────────────────────────────────

    async function loadPage(slug) {
        try {
            const page = await apiGet(`/api/wiki/pages/${encodeURIComponent(slug)}`);
            currentPageSlug = slug;
            showPage(page);
            renderPageList(); // Update active highlight
        } catch (e) {
            console.error('Failed to load wiki page:', e);
        }
    }

    function showPage(page) {
        document.getElementById('wiki-welcome').style.display = 'none';
        document.getElementById('wiki-query-response').style.display = 'none';
        const view = document.getElementById('wiki-page-view');
        view.style.display = '';

        document.getElementById('wiki-page-title').textContent = page.title;

        // Hide delete button for auto-generated Index page
        document.getElementById('wiki-delete-btn').style.display = page.slug === 'index' ? 'none' : '';

        // Meta
        const metaEl = document.getElementById('wiki-page-meta');
        const updated = page.updated_at ? new Date(page.updated_at).toLocaleDateString() : '';
        metaEl.innerHTML = `
            <span class="wiki-meta-badge">${escapeHtml(page.category)}</span>
            ${page.source_files && page.source_files.length ? `<span class="secondary">${page.source_files.length} sources</span>` : ''}
            <span class="secondary">Updated: ${updated}</span>
        `;

        // Render markdown content with wikilinks
        const rendered = renderWikiMarkdown(page.content);
        document.getElementById('wiki-page-content').innerHTML = rendered;

        // Backlinks
        const backlinksEl = document.getElementById('wiki-page-backlinks');
        if (page.backlinks && page.backlinks.length > 0) {
            backlinksEl.style.display = '';
            document.getElementById('wiki-backlinks-list').innerHTML = page.backlinks.map(slug =>
                `<a href="#" class="wiki-backlink" data-slug="${escapeHtml(slug)}">${escapeHtml(slug)}</a>`
            ).join(', ');
            backlinksEl.querySelectorAll('.wiki-backlink').forEach(a => {
                a.addEventListener('click', (e) => {
                    e.preventDefault();
                    loadPage(a.dataset.slug);
                });
            });
        } else {
            backlinksEl.style.display = 'none';
        }

        // Source References
        const refsEl = document.getElementById('wiki-page-references');
        const refsList = document.getElementById('wiki-references-list');
        if (page.source_files && page.source_files.length > 0) {
            refsEl.style.display = '';
            refsList.innerHTML = '';
            page.source_files.forEach((src, idx) => {
                const item = document.createElement('div');
                item.className = 'wiki-ref-item';
                const link = document.createElement('a');
                link.className = 'wiki-ref-link';
                const pages = src.pages && src.pages.length
                    ? ` (p.${src.pages.join(', ')})`
                    : '';
                link.textContent = `[${idx + 1}] ${src.filename || src.file_path}${pages}`;
                link.href = '#';
                link.addEventListener('click', (e) => {
                    e.preventDefault();
                    apiPost('/api/documents/open-by-path', { file_path: src.file_path });
                });
                item.appendChild(link);
                refsList.appendChild(item);
            });
        } else {
            refsEl.style.display = 'none';
        }
    }

    function renderWikiMarkdown(content) {
        // First render markdown
        let html = marked.parse(content || '');
        // Then process [[wikilinks]]
        html = html.replace(/\[\[(.+?)\]\]/g, (match, title) => {
            const slug = title.toLowerCase().replace(/[^\w\s-]/g, '').replace(/[\s_]+/g, '-').replace(/-+/g, '-').replace(/^-|-$/g, '');
            return `<a href="#" class="wikilink" data-slug="${escapeHtml(slug)}" title="${escapeHtml(title)}">${escapeHtml(title)}</a>`;
        });
        return html;
    }

    // Global click handler for wikilinks
    document.addEventListener('click', (e) => {
        const wl = e.target.closest('.wikilink');
        if (!wl) return;
        e.preventDefault();
        loadPage(wl.dataset.slug);
    });

    // Listen for cross-page navigation from chat
    document.addEventListener('wiki:navigate', (e) => {
        if (e.detail && e.detail.slug) {
            loadPage(e.detail.slug);
        }
    });

    // ── Search ───────────────────────────────────────────────────────────

    async function searchPages(query) {
        try {
            const results = await apiGet(`/api/wiki/search?q=${encodeURIComponent(query)}`);
            const container = document.getElementById('wiki-page-list');
            if (results.length === 0) {
                container.innerHTML = '<p class="secondary" style="padding:1rem;font-size:0.85rem;">No results found.</p>';
                return;
            }
            container.innerHTML = results.map(r => `
                <div class="wiki-page-item" data-slug="${escapeHtml(r.slug)}">
                    <div class="wiki-page-item-title">${escapeHtml(r.title)}</div>
                    <div class="wiki-page-item-meta">
                        <span class="wiki-page-item-cat">${escapeHtml(r.category)}</span>
                        ${r.score ? `<span class="wiki-score">${(r.score * 100).toFixed(0)}%</span>` : ''}
                        <span class="wiki-match-type">${r.match_type}</span>
                    </div>
                </div>
            `).join('');
            container.querySelectorAll('.wiki-page-item').forEach(el => {
                el.addEventListener('click', () => loadPage(el.dataset.slug));
            });
        } catch (e) {
            console.error('Wiki search failed:', e);
        }
    }

    // ── Query ────────────────────────────────────────────────────────────

    async function submitQuery() {
        const input = document.getElementById('wiki-query-input');
        const question = input.value.trim();
        if (!question || isQuerying) return;

        if (!await window.requireLmStudio()) return;

        isQuerying = true;
        queryAnswer = '';
        queryPagesUsed = [];

        const btn = document.getElementById('wiki-query-btn');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');

        // Show query response area
        document.getElementById('wiki-welcome').style.display = 'none';
        document.getElementById('wiki-page-view').style.display = 'none';
        const responseArea = document.getElementById('wiki-query-response');
        responseArea.style.display = '';
        document.getElementById('wiki-query-question').textContent = question;
        document.getElementById('wiki-query-content').innerHTML = '<p class="secondary">Searching wiki...</p>';
        document.getElementById('wiki-query-sources').style.display = 'none';
        document.getElementById('wiki-query-save-btn').style.display = 'none';

        try {
            const response = await fetch('/api/wiki/query', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ question, save_as_page: false }),
            });

            const reader = response.body.getReader();
            const decoder = new TextDecoder();
            let buffer = '';

            document.getElementById('wiki-query-content').innerHTML = '';

            while (true) {
                const { done, value } = await reader.read();
                if (done) break;

                buffer += decoder.decode(value, { stream: true });
                const lines = buffer.split('\n');
                buffer = lines.pop();

                for (const line of lines) {
                    if (line.startsWith('data: ')) {
                        const data = JSON.parse(line.slice(6));
                        if (data.token !== undefined) {
                            queryAnswer += data.token;
                            document.getElementById('wiki-query-content').innerHTML = renderWikiMarkdown(queryAnswer);
                        }
                    }
                    if (line.startsWith('event: done')) {
                        // Next data line has the done payload
                    }
                    if (line.startsWith('data: ') && line.includes('pages_used')) {
                        try {
                            const doneData = JSON.parse(line.slice(6));
                            if (doneData.pages_used) {
                                queryPagesUsed = doneData.pages_used;
                                showQuerySources(queryPagesUsed);
                            }
                        } catch (e) { /* not done event */ }
                    }
                }
            }

            // Show save button if we got an answer
            if (queryAnswer.trim()) {
                document.getElementById('wiki-query-save-btn').style.display = '';
            }
        } catch (e) {
            document.getElementById('wiki-query-content').innerHTML =
                `<p class="secondary">Error: ${escapeHtml(e.message)}</p>`;
        } finally {
            isQuerying = false;
            btn.disabled = false;
            btn.setAttribute('aria-busy', 'false');
            input.value = '';
        }
    }

    function showQuerySources(pages) {
        if (!pages || pages.length === 0) return;
        const el = document.getElementById('wiki-query-sources');
        el.style.display = '';
        el.innerHTML = '<strong>Wiki pages used:</strong> ' + pages.map(p =>
            `<a href="#" class="wikilink" data-slug="${escapeHtml(p.slug)}">${escapeHtml(p.title)}</a> (${(p.score * 100).toFixed(0)}%)`
        ).join(', ');
    }

    function closeQueryResponse() {
        document.getElementById('wiki-query-response').style.display = 'none';
        if (currentPageSlug) {
            loadPage(currentPageSlug);
        } else {
            document.getElementById('wiki-welcome').style.display = '';
            updateWikiReadiness();
        }
    }

    async function saveQueryAsPage() {
        if (!queryAnswer.trim()) return;
        const question = document.getElementById('wiki-query-question').textContent;
        try {
            const result = await apiPost('/api/wiki/pages', {
                title: question.substring(0, 80),
                content: queryAnswer,
                category: 'topic',
                summary: question.substring(0, 200),
            });
            document.getElementById('wiki-query-save-btn').style.display = 'none';
            loadPages();
            loadPage(result.slug);
        } catch (e) {
            console.error('Failed to save query as page:', e);
        }
    }

    // ── Ingest (Dialog) ──────────────────────────────────────────────────

    async function openIngestDialog() {
        if (!await window.requireLmStudio()) return;

        const dlg = document.getElementById('wiki-ingest-dialog');
        const tagsContainer = document.getElementById('wiki-ingest-tags');
        selectedIngestTags.clear();

        // Load tags
        try {
            allTags = await apiGet('/api/tags');
        } catch (e) {
            allTags = [];
        }

        tagsContainer.innerHTML = allTags.map(t => `
            <label class="wiki-ingest-tag-label">
                <input type="checkbox" value="${escapeHtml(t.name)}" class="wiki-ingest-tag-cb">
                <span class="tag-badge" style="border-color:${t.color};color:${t.color};">${escapeHtml(t.display_name)}</span>
                <span class="secondary" style="font-size:0.8rem;">(${t.file_count} files)</span>
            </label>
        `).join('');

        tagsContainer.querySelectorAll('.wiki-ingest-tag-cb').forEach(cb => {
            cb.addEventListener('change', () => {
                if (cb.checked) selectedIngestTags.add(cb.value);
                else selectedIngestTags.delete(cb.value);
            });
        });

        document.getElementById('wiki-ingest-progress').style.display = 'none';
        document.getElementById('wiki-ingest-start-btn').disabled = false;
        dlg.showModal();
    }

    async function startIngest() {
        if (selectedIngestTags.size === 0) {
            alert('Select at least one tag to ingest from.');
            return;
        }

        // Save params for resume
        lastIngestTagNames = Array.from(selectedIngestTags);
        lastIngestFileIds = [];

        const btn = document.getElementById('wiki-ingest-start-btn');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');
        document.getElementById('wiki-ingest-progress').style.display = '';

        // Show stop button, hide start
        const stopBtn = document.getElementById('wiki-ingest-stop-btn');
        if (stopBtn) {
            stopBtn.style.display = '';
            stopBtn.disabled = false;
            stopBtn.removeAttribute('aria-busy');
            stopBtn.textContent = 'Stop';
        }

        try {
            await apiPost('/api/wiki/ingest', {
                tag_names: lastIngestTagNames,
            });
            pollInitProgress();
        } catch (e) {
            btn.disabled = false;
            btn.setAttribute('aria-busy', 'false');
            if (stopBtn) stopBtn.style.display = 'none';
            alert('Ingest failed: ' + e.message);
        }
    }

    // ── Lint ──────────────────────────────────────────────────────────────

    let lintFixPollTimer = null;

    async function runLint() {
        if (!await window.requireLmStudio()) return;

        const dlg = document.getElementById('wiki-lint-dialog');
        const content = document.getElementById('wiki-lint-content');
        content.innerHTML = '<p class="secondary">Running health check...</p>';
        document.getElementById('wiki-lint-apply-btn').style.display = 'none';
        document.getElementById('wiki-lint-stop-btn').style.display = 'none';
        dlg.showModal();

        try {
            const result = await apiPost('/api/wiki/lint');
            renderLintResults(result);
        } catch (e) {
            content.innerHTML = `<p>Error: ${escapeHtml(e.message || 'Lint failed')}</p>`;
        }
    }

    function renderLintResults(result) {
        const content = document.getElementById('wiki-lint-content');
        const applyBtn = document.getElementById('wiki-lint-apply-btn');
        const stopBtn = document.getElementById('wiki-lint-stop-btn');
        applyBtn.style.display = 'none';
        stopBtn.style.display = 'none';

        if (result.error) {
            content.innerHTML = `<p class="secondary">${escapeHtml(result.error)}</p>`;
            return;
        }

        let html = '';
        let hasItems = false;

        // Select all toggle
        html += '<label style="margin-bottom:0.5rem;display:inline-block"><input type="checkbox" id="wiki-lint-select-all"> <strong>Select All</strong></label>';

        if (result.orphan_pages && result.orphan_pages.length > 0) {
            hasItems = true;
            html += '<h4>Orphan Pages (no inbound links)</h4><ul style="list-style:none;padding-left:0">';
            result.orphan_pages.forEach(s => {
                html += `<li><label><input type="checkbox" class="lint-fix-cb" data-fix-type="orphan_pages" data-value="${escapeHtml(s)}"> <a href="#" class="wikilink" data-slug="${escapeHtml(s)}">${escapeHtml(s)}</a></label></li>`;
            });
            html += '</ul>';
        }

        if (result.missing_pages && result.missing_pages.length > 0) {
            hasItems = true;
            html += '<h4>Missing Pages (referenced but don\'t exist)</h4><ul style="list-style:none;padding-left:0">';
            result.missing_pages.forEach(t => {
                html += `<li><label><input type="checkbox" class="lint-fix-cb" data-fix-type="missing_pages" data-value="${escapeHtml(t)}"> ${escapeHtml(t)}</label></li>`;
            });
            html += '</ul>';
        }

        if (result.stale_pages && result.stale_pages.length > 0) {
            hasItems = true;
            html += '<h4>Stale Pages (may need updating)</h4><ul style="list-style:none;padding-left:0">';
            result.stale_pages.forEach(s => {
                html += `<li><label><input type="checkbox" class="lint-fix-cb" data-fix-type="stale_pages" data-value="${escapeHtml(s)}"> <a href="#" class="wikilink" data-slug="${escapeHtml(s)}">${escapeHtml(s)}</a></label></li>`;
            });
            html += '</ul>';
        }

        if (result.missing_crossrefs && result.missing_crossrefs.length > 0) {
            hasItems = true;
            html += '<h4>Missing Cross-References</h4><ul style="list-style:none;padding-left:0">';
            result.missing_crossrefs.forEach(r => {
                const val = escapeHtml(JSON.stringify({from_slug: r.from_slug, should_link_to: r.should_link_to}));
                html += `<li><label><input type="checkbox" class="lint-fix-cb" data-fix-type="missing_crossrefs" data-value="${val}"> ${escapeHtml(r.from_slug)} &rarr; ${escapeHtml(r.should_link_to)}</label></li>`;
            });
            html += '</ul>';
        }

        if (result.suggested_pages && result.suggested_pages.length > 0) {
            hasItems = true;
            html += '<h4>Suggested New Pages</h4><ul style="list-style:none;padding-left:0">';
            result.suggested_pages.forEach(t => {
                html += `<li><label><input type="checkbox" class="lint-fix-cb" data-fix-type="suggested_pages" data-value="${escapeHtml(t)}"> ${escapeHtml(t)}</label></li>`;
            });
            html += '</ul>';
        }

        if (!hasItems) {
            content.innerHTML = '<p class="secondary">Wiki looks healthy! No issues found.</p>';
            return;
        }

        content.innerHTML = html;

        // Wire up select all toggle
        const selectAll = document.getElementById('wiki-lint-select-all');
        selectAll.addEventListener('change', () => {
            content.querySelectorAll('.lint-fix-cb').forEach(cb => { cb.checked = selectAll.checked; });
            updateApplyBtnState();
        });

        // Wire up individual checkboxes to update button state
        content.querySelectorAll('.lint-fix-cb').forEach(cb => {
            cb.addEventListener('change', updateApplyBtnState);
        });

        // Show apply button (disabled initially)
        applyBtn.style.display = '';
        applyBtn.disabled = true;
    }

    function updateApplyBtnState() {
        const checked = document.querySelectorAll('.lint-fix-cb:checked').length;
        const applyBtn = document.getElementById('wiki-lint-apply-btn');
        applyBtn.disabled = checked === 0;
        applyBtn.textContent = checked > 0 ? `Apply Selected Fixes (${checked})` : 'Apply Selected Fixes';
    }

    async function applyLintFixes() {
        const payload = {
            orphan_pages: [],
            missing_pages: [],
            stale_pages: [],
            missing_crossrefs: [],
            suggested_pages: [],
        };

        document.querySelectorAll('.lint-fix-cb:checked').forEach(cb => {
            const type = cb.dataset.fixType;
            const val = cb.dataset.value;
            if (type === 'missing_crossrefs') {
                payload.missing_crossrefs.push(JSON.parse(val));
            } else {
                payload[type].push(val);
            }
        });

        const total = Object.values(payload).reduce((s, a) => s + a.length, 0);
        if (total === 0) return;

        const content = document.getElementById('wiki-lint-content');
        const applyBtn = document.getElementById('wiki-lint-apply-btn');
        const stopBtn = document.getElementById('wiki-lint-stop-btn');

        applyBtn.style.display = 'none';
        stopBtn.style.display = '';

        content.innerHTML =
            '<p class="secondary">Applying fixes...</p>' +
            '<progress id="wiki-lint-fix-bar" value="0" max="100"></progress>' +
            '<p id="wiki-lint-fix-text" class="secondary" style="margin-top:0.5rem">Starting...</p>';

        try {
            await apiPost('/api/wiki/lint/apply', payload);
            pollLintFixProgress();
        } catch (e) {
            content.innerHTML = `<p>Error: ${escapeHtml(e.message || 'Failed to start fixes')}</p>`;
            stopBtn.style.display = 'none';
        }
    }

    function pollLintFixProgress() {
        if (lintFixPollTimer) clearInterval(lintFixPollTimer);
        lintFixPollTimer = setInterval(async () => {
            try {
                const status = await apiGet('/api/wiki/lint/apply/status');
                const bar = document.getElementById('wiki-lint-fix-bar');
                const text = document.getElementById('wiki-lint-fix-text');
                if (!bar || !text) return;

                const pct = status.total_fixes > 0
                    ? Math.round((status.processed_fixes / status.total_fixes) * 100) : 0;
                bar.value = pct;

                const progressMsg = status.total_fixes > 0
                    ? `${status.processed_fixes}/${status.total_fixes} fixes processed. ` +
                      `${status.pages_created} created, ${status.pages_updated} updated.` +
                      (status.current_fix ? ` Current: ${status.current_fix}` : '')
                    : 'Starting...';
                text.textContent = progressMsg;

                if (status.phase === 'done' || status.phase === 'stopped') {
                    clearInterval(lintFixPollTimer);
                    lintFixPollTimer = null;
                    document.getElementById('wiki-lint-stop-btn').style.display = 'none';

                    let summary = status.phase === 'stopped'
                        ? '<p><strong>Fixes stopped.</strong></p>'
                        : '<p><strong>Fixes applied successfully.</strong></p>';
                    summary += `<p>Created ${status.pages_created} pages, updated ${status.pages_updated} pages.</p>`;
                    if (status.errors && status.errors.length > 0) {
                        summary += '<details><summary>Errors (' + status.errors.length + ')</summary><ul>';
                        status.errors.forEach(e => { summary += `<li>${escapeHtml(e)}</li>`; });
                        summary += '</ul></details>';
                    }
                    summary += '<button id="wiki-lint-rerun-btn" class="outline" style="margin-top:0.5rem">Re-run Health Check</button>';

                    const content = document.getElementById('wiki-lint-content');
                    content.innerHTML = summary;

                    document.getElementById('wiki-lint-rerun-btn').addEventListener('click', () => {
                        document.getElementById('wiki-lint-dialog').close();
                        runLint();
                    });

                    loadPages();
                    updateWikiReadiness();
                }
            } catch (e) {
                console.error('Lint fix poll error:', e);
            }
        }, 1500);
    }

    async function stopLintFix() {
        try {
            await apiPost('/api/wiki/lint/apply/stop');
        } catch (e) {
            console.error('Failed to stop lint fix:', e);
        }
    }

    // ── New Page ─────────────────────────────────────────────────────────

    function openNewPageDialog() {
        document.getElementById('wiki-new-title').value = '';
        document.getElementById('wiki-new-category').value = 'general';
        document.getElementById('wiki-new-content').value = '';
        document.getElementById('wiki-new-page-dialog').showModal();
    }

    async function createNewPage() {
        const title = document.getElementById('wiki-new-title').value.trim();
        const category = document.getElementById('wiki-new-category').value;
        const content = document.getElementById('wiki-new-content').value;

        if (!title) {
            alert('Title is required.');
            return;
        }

        try {
            const result = await apiPost('/api/wiki/pages', { title, category, content });
            document.getElementById('wiki-new-page-dialog').close();
            loadPages();
            loadPage(result.slug);
        } catch (e) {
            alert('Failed to create page: ' + e.message);
        }
    }

    // ── Delete Page ──────────────────────────────────────────────────────

    function confirmDeletePage() {
        if (!currentPageSlug) return;
        document.getElementById('wiki-delete-dialog').showModal();
    }

    async function deletePage() {
        if (!currentPageSlug) return;
        try {
            await apiDelete(`/api/wiki/pages/${encodeURIComponent(currentPageSlug)}`);
            document.getElementById('wiki-delete-dialog').close();
            currentPageSlug = null;
            document.getElementById('wiki-page-view').style.display = 'none';
            document.getElementById('wiki-welcome').style.display = '';
            loadPages();
            updateWikiReadiness();
        } catch (e) {
            alert('Failed to delete page: ' + e.message);
        }
    }

})();
