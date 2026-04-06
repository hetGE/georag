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

        // Ingest dialog
        document.querySelectorAll('.close-wiki-ingest').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('wiki-ingest-dialog').close());
        });
        document.getElementById('wiki-ingest-start-btn').addEventListener('click', startIngest);

        // Lint dialog
        document.querySelectorAll('.close-wiki-lint').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('wiki-lint-dialog').close());
        });

        // New page dialog
        document.querySelectorAll('.close-wiki-new-page').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('wiki-new-page-dialog').close());
        });
        document.getElementById('wiki-new-create-btn').addEventListener('click', createNewPage);

        // Delete dialog
        document.querySelector('#wiki-delete-dialog .dialog-cancel').addEventListener('click', () => {
            document.getElementById('wiki-delete-dialog').close();
        });
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
    window.updateWikiQueryState = updateQueryInputState;

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
        }
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
            ingestBtn.innerHTML = `Rebuild Wiki for Tag <span class="wiki-update-badge">${stats.files_since_last_ingest}</span>`;
            ingestBtn.title = `${stats.files_since_last_ingest} new files since last ingest`;
        } else {
            ingestBtn.textContent = 'Rebuild Wiki for Tag';
            ingestBtn.title = 'Rebuild wiki from tagged documents';
        }
    }

    function updateActionButtonStates(stats) {
        const isProcessing = stats.library_is_processing;
        const ingestBtn = document.getElementById('wiki-ingest-btn');
        const lintBtn = document.getElementById('wiki-lint-btn');

        // Disable ingest during library processing
        if (isProcessing) {
            ingestBtn.disabled = true;
            ingestBtn.title = 'Library is still processing documents...';
            lintBtn.disabled = true;
            lintBtn.title = 'Library is still processing documents...';
        } else {
            ingestBtn.disabled = false;
            lintBtn.disabled = false;
            lintBtn.title = 'Health check';
        }
    }

    // ── Initialize Wiki (one-click ingest all) ───────────────────────────

    async function initializeWiki() {
        if (!await window.requireLmStudio()) return;

        // Show ingesting state
        document.querySelectorAll('#wiki-welcome .wiki-state').forEach(el => {
            el.style.display = 'none';
        });
        document.getElementById('wiki-state-ingesting').style.display = '';
        document.getElementById('wiki-init-progress-text').textContent = 'Starting wiki initialization...';
        document.getElementById('wiki-init-progress-bar').value = 0;

        try {
            // Ingest all processed files (empty tag_names = all files)
            await apiPost('/api/wiki/ingest', { tag_names: [], file_ids: [] });
            pollInitProgress();
        } catch (e) {
            document.getElementById('wiki-init-progress-text').textContent =
                'Error: ' + (e.message || 'Failed to start ingest');
        }
    }

    function pollInitProgress() {
        // Reuse the ingest poll timer mechanism but update inline progress
        if (ingestPollTimer) clearInterval(ingestPollTimer);
        ingestPollTimer = setInterval(async () => {
            try {
                const status = await apiGet('/api/wiki/ingest/status');

                // Update inline progress in welcome area
                const bar = document.getElementById('wiki-init-progress-bar');
                const text = document.getElementById('wiki-init-progress-text');

                if (bar && text) {
                    if (status.total_sources > 0) {
                        const pct = Math.round((status.processed_sources / status.total_sources) * 100);
                        bar.value = pct;
                        text.textContent = `${status.processed_sources}/${status.total_sources} sources processed. ` +
                            `${status.pages_created} created, ${status.pages_updated} updated.` +
                            (status.current_source ? ` Current: ${status.current_source}` : '');
                    }
                }

                // Also update dialog progress if it's open
                const dlgBar = document.getElementById('wiki-ingest-progress-bar');
                const dlgText = document.getElementById('wiki-ingest-progress-text');
                if (dlgBar && dlgText && status.total_sources > 0) {
                    const pct = Math.round((status.processed_sources / status.total_sources) * 100);
                    dlgBar.value = pct;
                    dlgText.textContent = `${status.processed_sources}/${status.total_sources} sources processed. ` +
                        `${status.pages_created} created, ${status.pages_updated} updated.` +
                        (status.current_source ? ` Current: ${status.current_source}` : '');
                }

                if (!status.is_running) {
                    clearInterval(ingestPollTimer);
                    ingestPollTimer = null;

                    // Update dialog button if open
                    const dlgBtn = document.getElementById('wiki-ingest-start-btn');
                    if (dlgBtn) {
                        dlgBtn.disabled = false;
                        dlgBtn.setAttribute('aria-busy', 'false');
                        dlgBtn.textContent = 'Done!';
                        if (dlgText) {
                            dlgText.textContent = `Complete. ${status.pages_created} created, ${status.pages_updated} updated.`;
                            if (status.errors && status.errors.length > 0) {
                                dlgText.textContent += ` ${status.errors.length} errors.`;
                            }
                        }
                        setTimeout(() => { dlgBtn.textContent = 'Start Rebuild'; }, 2000);
                    }

                    // Refresh pages and readiness state
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

        const btn = document.getElementById('wiki-ingest-start-btn');
        btn.disabled = true;
        btn.setAttribute('aria-busy', 'true');
        document.getElementById('wiki-ingest-progress').style.display = '';

        try {
            await apiPost('/api/wiki/ingest', {
                tag_names: Array.from(selectedIngestTags),
            });
            // Poll for status (unified poller handles both inline + dialog)
            pollInitProgress();
        } catch (e) {
            btn.disabled = false;
            btn.setAttribute('aria-busy', 'false');
            alert('Ingest failed: ' + e.message);
        }
    }

    // ── Lint ──────────────────────────────────────────────────────────────

    async function runLint() {
        if (!await window.requireLmStudio()) return;

        const dlg = document.getElementById('wiki-lint-dialog');
        const content = document.getElementById('wiki-lint-content');
        content.innerHTML = '<p class="secondary">Running health check...</p>';
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
        if (result.error) {
            content.innerHTML = `<p class="secondary">${escapeHtml(result.error)}</p>`;
            return;
        }

        let html = '';

        if (result.orphan_pages && result.orphan_pages.length > 0) {
            html += '<h4>Orphan Pages (no inbound links)</h4><ul>';
            result.orphan_pages.forEach(s => {
                html += `<li><a href="#" class="wikilink" data-slug="${escapeHtml(s)}">${escapeHtml(s)}</a></li>`;
            });
            html += '</ul>';
        }

        if (result.missing_pages && result.missing_pages.length > 0) {
            html += '<h4>Missing Pages (referenced but don\'t exist)</h4><ul>';
            result.missing_pages.forEach(t => { html += `<li>${escapeHtml(t)}</li>`; });
            html += '</ul>';
        }

        if (result.stale_pages && result.stale_pages.length > 0) {
            html += '<h4>Stale Pages (may need updating)</h4><ul>';
            result.stale_pages.forEach(s => {
                html += `<li><a href="#" class="wikilink" data-slug="${escapeHtml(s)}">${escapeHtml(s)}</a></li>`;
            });
            html += '</ul>';
        }

        if (result.missing_crossrefs && result.missing_crossrefs.length > 0) {
            html += '<h4>Missing Cross-References</h4><ul>';
            result.missing_crossrefs.forEach(r => {
                html += `<li>${escapeHtml(r.from_slug)} should link to ${escapeHtml(r.should_link_to)}</li>`;
            });
            html += '</ul>';
        }

        if (result.suggested_pages && result.suggested_pages.length > 0) {
            html += '<h4>Suggested New Pages</h4><ul>';
            result.suggested_pages.forEach(t => { html += `<li>${escapeHtml(t)}</li>`; });
            html += '</ul>';
        }

        if (!html) {
            html = '<p class="secondary">Wiki looks healthy! No issues found.</p>';
        }

        content.innerHTML = html;
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
