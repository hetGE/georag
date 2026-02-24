// GeoRAG Chat Interface

let currentConversationId = null;
let selectedTags = new Set();
let allTags = [];
let isStreaming = false;

document.addEventListener('DOMContentLoaded', async () => {
    await loadTags();
    await loadConversations();
    await checkOnboarding();

    document.getElementById('chat-form').addEventListener('submit', handleSubmit);
    document.getElementById('new-chat-btn').addEventListener('click', newChat);

    // Enter to send, Shift+Enter for newline
    document.getElementById('chat-input').addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            handleSubmit(e);
        }
    });
});

async function loadTags() {
    allTags = await apiGet('/api/tags');
    const container = document.getElementById('tag-badges');
    container.innerHTML = '';

    allTags.forEach(tag => {
        const badge = createTagBadge(tag, false, toggleTag);
        container.appendChild(badge);
    });
}

function toggleTag(tag, badge) {
    if (selectedTags.has(tag.name)) {
        selectedTags.delete(tag.name);
        badge.classList.remove('selected');
        badge.style.color = tag.color;
        badge.style.backgroundColor = 'transparent';
    } else {
        selectedTags.add(tag.name);
        badge.classList.add('selected');
        badge.style.color = '#fff';
        badge.style.backgroundColor = tag.color;
    }
}

async function loadConversations() {
    const conversations = await apiGet('/api/conversations');
    const list = document.getElementById('conversation-list');

    if (!conversations.length) {
        list.innerHTML = '<p class="secondary" style="font-size:0.85rem;padding:0.5rem;">No conversations yet</p>';
        return;
    }

    list.innerHTML = '';
    conversations.forEach(conv => {
        const item = document.createElement('div');
        item.className = 'conversation-item' + (conv.id === currentConversationId ? ' active' : '');
        item.innerHTML = `
            <span class="conv-title">${escapeHtml(conv.title)}</span>
            <span class="conv-date">${formatDate(conv.updated_at)}</span>
            <span class="conv-delete" title="Delete">&#x2715;</span>
        `;
        item.addEventListener('click', (e) => {
            if (e.target.classList.contains('conv-delete')) {
                deleteConversation(conv.id);
            } else {
                loadConversation(conv.id);
            }
        });
        list.appendChild(item);
    });
}

async function loadConversation(convId) {
    currentConversationId = convId;
    const data = await apiGet(`/api/conversations/${convId}`);
    const messagesEl = document.getElementById('chat-messages');
    messagesEl.innerHTML = '';

    // Restore selected tags
    selectedTags = new Set(data.selected_tags || []);
    document.querySelectorAll('.tag-badge').forEach(badge => {
        const tag = allTags.find(t => t.name === badge.dataset.tagName);
        if (tag && selectedTags.has(tag.name)) {
            badge.classList.add('selected');
            badge.style.color = '#fff';
            badge.style.backgroundColor = tag.color;
        } else if (tag) {
            badge.classList.remove('selected');
            badge.style.color = tag.color;
            badge.style.backgroundColor = 'transparent';
        }
    });

    data.messages.forEach(msg => {
        appendMessage(msg.role, msg.content, msg.sources);
    });

    await loadConversations();
    scrollToBottom();
}

async function deleteConversation(convId) {
    await apiDelete(`/api/conversations/${convId}`);
    if (convId === currentConversationId) {
        newChat();
    }
    await loadConversations();
}

function newChat() {
    currentConversationId = null;
    document.getElementById('chat-messages').innerHTML = `
        <div class="chat-welcome">
            <h2>GeoRAG</h2>
            <p>Geotechnical Engineering RAG Assistant</p>
            <p class="secondary">Select tags above to focus your search, then ask a question.</p>
        </div>
    `;
    loadConversations();
}

async function handleSubmit(e) {
    e.preventDefault();
    const input = document.getElementById('chat-input');
    const message = input.value.trim();
    if (!message || isStreaming) return;

    // Clear welcome message
    const welcome = document.querySelector('.chat-welcome');
    if (welcome) welcome.remove();

    // Show user message
    appendMessage('user', message);
    input.value = '';

    // Start streaming
    isStreaming = true;
    document.getElementById('send-btn').disabled = true;
    const assistantDiv = appendMessage('assistant', '', null, true);
    const contentEl = assistantDiv.querySelector('.message-content');

    try {
        const response = await fetch('/api/chat', {
            method: 'POST',
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({
                message: message,
                conversation_id: currentConversationId,
                tag_names: Array.from(selectedTags),
            }),
        });

        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';
        let fullText = '';

        while (true) {
            const {done, value} = await reader.read();
            if (done) break;

            buffer += decoder.decode(value, {stream: true});
            const lines = buffer.split('\n');
            buffer = lines.pop();

            for (const line of lines) {
                if (line.startsWith('data: ')) {
                    const dataStr = line.slice(6);
                    try {
                        const data = JSON.parse(dataStr);

                        if (data.token !== undefined) {
                            fullText += data.token;
                            contentEl.innerHTML = marked.parse(fullText);
                            scrollToBottom();
                        }
                    } catch {}
                }
                if (line.startsWith('event: done')) {
                    // Next data line has sources info
                }
                if (line.startsWith('event: error')) {
                    // Next data line has error
                }
                if (line.startsWith('data: ') && line.includes('"conversation_id"')) {
                    try {
                        const doneData = JSON.parse(line.slice(6));
                        if (doneData.conversation_id) {
                            currentConversationId = doneData.conversation_id;
                        }
                        if (doneData.sources && doneData.sources.length) {
                            appendSources(assistantDiv, doneData.sources);
                        }
                    } catch {}
                }
            }
        }

    } catch (err) {
        contentEl.textContent = 'Error: ' + err.message;
    }

    isStreaming = false;
    document.getElementById('send-btn').disabled = false;
    await loadConversations();
    scrollToBottom();
}

function appendMessage(role, content, sources = null, streaming = false) {
    const messagesEl = document.getElementById('chat-messages');
    const div = document.createElement('div');
    div.className = `message ${role}`;

    const contentEl = document.createElement('div');
    contentEl.className = 'message-content';

    if (streaming) {
        contentEl.innerHTML = '<span class="loading-dots">Thinking</span>';
    } else if (role === 'assistant') {
        contentEl.innerHTML = marked.parse(content);
    } else {
        contentEl.textContent = content;
    }

    div.appendChild(contentEl);

    if (sources && sources.length) {
        appendSources(div, sources);
    }

    messagesEl.appendChild(div);
    scrollToBottom();
    return div;
}

function appendSources(messageDiv, sources) {
    // Remove existing sources
    const existing = messageDiv.querySelector('.message-sources');
    if (existing) existing.remove();

    const sourcesEl = document.createElement('details');
    sourcesEl.className = 'message-sources';
    sourcesEl.innerHTML = `<summary>Sources (${sources.length})</summary>`;

    sources.forEach(src => {
        const item = document.createElement('div');
        item.className = 'source-item';
        const page = src.page ? ` (p.${src.page})` : '';
        const score = src.score ? ` [${(src.score * 100).toFixed(0)}%]` : '';
        item.textContent = `${src.filename || src.file_path}${page}${score}`;
        sourcesEl.appendChild(item);
    });

    messageDiv.appendChild(sourcesEl);
}

function scrollToBottom() {
    const el = document.getElementById('chat-messages');
    el.scrollTop = el.scrollHeight;
}

// --- Onboarding Wizard ---

let onboardingPollInterval = null;

async function checkOnboarding() {
    try {
        const status = await apiGet('/api/processing/onboarding-status');
        if (status.dismissed && status.phase !== 'processing') {
            hideOnboarding();
            return;
        }
        if (status.phase === 'complete') {
            hideOnboarding();
            return;
        }
        renderOnboardingStep(status);
    } catch {
        // API not available
    }
}

function hideOnboarding() {
    const el = document.getElementById('onboarding');
    el.style.display = 'none';
    stopOnboardingPolling();
}

function showOnboarding() {
    document.getElementById('onboarding').style.display = 'block';
}

function renderOnboardingStep(status) {
    const el = document.getElementById('onboarding');

    if (status.phase === 'not_started') {
        renderScanStep(el);
    } else if (status.phase === 'scanned') {
        renderProcessStep(el, status);
    } else if (status.phase === 'processing') {
        renderProgressStep(el, status);
        startOnboardingPolling();
    } else if (status.phase === 'complete') {
        renderCompleteStep(el, status);
    }

    showOnboarding();
}

function renderScanStep(el) {
    el.innerHTML = `
        <h3 class="onboarding-title">Welcome to GeoRAG</h3>
        <p class="onboarding-description">
            Let's index your geotechnical document library. First, we'll scan your Engineering folder to discover all documents.
        </p>
        <div class="onboarding-actions">
            <button id="onboarding-scan-btn">Scan Library</button>
        </div>
    `;
    document.getElementById('onboarding-scan-btn').addEventListener('click', onboardingScan);
}

async function onboardingScan() {
    const btn = document.getElementById('onboarding-scan-btn');
    btn.disabled = true;
    btn.setAttribute('aria-busy', 'true');
    btn.textContent = 'Scanning...';

    try {
        await apiPost('/api/processing/scan');
        await checkOnboarding();
    } catch {
        btn.textContent = 'Scan Failed — Retry';
        btn.disabled = false;
        btn.setAttribute('aria-busy', 'false');
    }
}

function renderProcessStep(el, status) {
    const extSummary = formatExtensionSummary(status.by_extension || {});
    const remaining = status.new_files;
    const alreadyProcessed = status.processed_files;
    const resumeLabel = alreadyProcessed > 0 ? 'Resume Processing' : 'Start Processing';

    el.innerHTML = `
        <h3 class="onboarding-title">Library Scanned</h3>
        <p class="onboarding-description">
            Found <strong>${status.total_files.toLocaleString()}</strong> files${extSummary}.
        </p>
        <p class="onboarding-description">
            ${remaining.toLocaleString()} files to process.
            We'll extract text, auto-tag by topic, and build the search index.
            This may take a while — you can stop and resume anytime.
        </p>
        ${alreadyProcessed > 0 ? `<p class="onboarding-stats">${alreadyProcessed.toLocaleString()} files already processed.</p>` : ''}
        <div class="onboarding-actions">
            <button id="onboarding-process-btn">${resumeLabel}</button>
            <button id="onboarding-skip-btn" class="outline secondary">Skip for Now</button>
        </div>
    `;
    document.getElementById('onboarding-process-btn').addEventListener('click', onboardingProcess);
    document.getElementById('onboarding-skip-btn').addEventListener('click', onboardingDismiss);
}

async function onboardingProcess() {
    const btn = document.getElementById('onboarding-process-btn');
    btn.disabled = true;
    btn.setAttribute('aria-busy', 'true');

    const result = await apiPost('/api/processing/start', {});
    if (result.error) {
        alert(result.error);
        btn.disabled = false;
        btn.setAttribute('aria-busy', 'false');
        return;
    }
    await checkOnboarding();
}

async function onboardingDismiss() {
    await apiPost('/api/processing/onboarding-dismiss');
    hideOnboarding();
}

function renderProgressStep(el, status) {
    const total = status.total_files;
    const processed = status.processed_files;
    const pct = total > 0 ? Math.round((processed / total) * 100) : 0;

    el.innerHTML = `
        <h3 class="onboarding-title">Processing Documents...</h3>
        <div class="onboarding-progress">
            <progress value="${pct}" max="100"></progress>
            <span class="onboarding-progress-text">${processed.toLocaleString()} / ${total.toLocaleString()} files (${pct}%)</span>
        </div>
        ${status.failed_files > 0 ? `<p class="onboarding-stats">${status.failed_files} failed</p>` : ''}
        <p class="onboarding-description" style="font-size:0.85rem;">You can close this and come back — progress is saved automatically.</p>
        <div class="onboarding-actions">
            <button id="onboarding-stop-btn" class="outline secondary">Stop</button>
        </div>
    `;
    document.getElementById('onboarding-stop-btn').addEventListener('click', onboardingStop);
}

async function onboardingStop() {
    await apiPost('/api/processing/stop');
    stopOnboardingPolling();
    // Wait a moment for processor to stop, then refresh
    setTimeout(() => checkOnboarding(), 2000);
}

function renderCompleteStep(el, status) {
    stopOnboardingPolling();
    el.innerHTML = `
        <h3 class="onboarding-title">Library Ready</h3>
        <p class="onboarding-description">
            ${status.processed_files.toLocaleString()} files processed${status.failed_files > 0 ? `, ${status.failed_files} failed` : ''}.
            Your document library is ready for searching.
        </p>
        <div class="onboarding-actions">
            <button id="onboarding-done-btn">Start Chatting</button>
        </div>
    `;
    document.getElementById('onboarding-done-btn').addEventListener('click', async () => {
        await apiPost('/api/processing/onboarding-dismiss');
        hideOnboarding();
        document.getElementById('chat-input').focus();
    });
}

function startOnboardingPolling() {
    if (onboardingPollInterval) return;
    onboardingPollInterval = setInterval(async () => {
        try {
            const status = await apiGet('/api/processing/onboarding-status');
            if (status.phase === 'processing') {
                renderProgressStep(document.getElementById('onboarding'), status);
            } else {
                stopOnboardingPolling();
                renderOnboardingStep(status);
            }
        } catch {
            // ignore
        }
    }, 2000);
}

function stopOnboardingPolling() {
    if (onboardingPollInterval) {
        clearInterval(onboardingPollInterval);
        onboardingPollInterval = null;
    }
}

function formatExtensionSummary(byExt) {
    const entries = Object.entries(byExt);
    if (!entries.length) return '';
    const parts = entries.slice(0, 5).map(([ext, count]) => {
        const label = ext ? ext.toUpperCase() : 'other';
        return `${count.toLocaleString()} ${label}`;
    });
    return ' (' + parts.join(', ') + ')';
}
