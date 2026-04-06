// geoRAG Chat Interface

// Render markdown with LaTeX math support.
// Extracts $$...$$ (display) and $...$ (inline) blocks before markdown
// parsing so that underscores/asterisks inside formulas aren't mangled,
// then renders them with KaTeX after marked has run.
function renderContent(text) {
    const mathBlocks = [];

    let processed = text;

    // Protect display math first ($$...$$)
    processed = processed.replace(/\$\$([\s\S]*?)\$\$/g, (_m, math) => {
        const idx = mathBlocks.length;
        mathBlocks.push({ math, display: true });
        return `\x00MATH${idx}\x00`;
    });

    // Protect inline math ($...$) — single $ must not span newlines
    processed = processed.replace(/\$([^\$\n]+?)\$/g, (_m, math) => {
        const idx = mathBlocks.length;
        mathBlocks.push({ math, display: false });
        return `\x00MATH${idx}\x00`;
    });

    let html = marked.parse(processed);

    // Replace placeholders with KaTeX-rendered HTML
    html = html.replace(/\x00MATH(\d+)\x00/g, (_m, idx) => {
        const block = mathBlocks[parseInt(idx)];
        try {
            return katex.renderToString(block.math.trim(), {
                displayMode: block.display,
                throwOnError: false,
            });
        } catch {
            return block.display ? `$$${block.math}$$` : `$${block.math}$`;
        }
    });

    return html;
}

let currentConversationId = null;
let selectedTags = new Set();
let allTags = [];
let isStreaming = false;
let remoteStreaming = false;
let isWelcomeState = true;
let pendingDeleteConvId = null;
let isTrashViewOpen = false;

// Retrieval depth settings (persisted in localStorage)
let currentDepth = localStorage.getItem('georag-depth') || 'optimal';
const DEPTH_LEVELS = [
    { key: 'quick',    label: 'Quick and less demanding',  topK: 5,  maxCtx: 8,   desc: '5 sources per topic, 8 max context chunks' },
    { key: 'optimal',  label: 'Optimal and balanced',      topK: 10, maxCtx: 16,  desc: '10 sources per topic, 16 max context chunks' },
    { key: 'deep',     label: 'Deep and demanding',        topK: 20, maxCtx: 32,  desc: '20 sources per topic, 32 max context chunks' },
    { key: 'deeper',   label: 'Deeper and very demanding', topK: 50, maxCtx: 80,  desc: '50 sources per topic, 80 max context chunks' },
    { key: 'ludicrous',label: 'Ludicrous',                 topK: 100,maxCtx: 200, desc: '100 sources per topic, 200 max context chunks' },
];

// Cross-tab streaming lock via BroadcastChannel (status only, no token relay)
const streamingChannel = new BroadcastChannel('chat-streaming');
let mirrorEventSource = null;
let activeAbortController = null;
let remoteStopHandler = null;

streamingChannel.onmessage = (e) => {
    const msg = e.data;

    // Handle stop-request from mirror tab
    if (msg.type === 'stop-request') {
        if (activeAbortController) {
            activeAbortController.abort();
        }
        apiPost('/api/chat/stop');
        return;
    }

    if (msg.streaming) {
        remoteStreaming = true;
        document.body.classList.add('chat-streaming');
        document.getElementById('new-chat-btn').disabled = true;
        document.querySelector('.chat-sidebar').classList.add('streaming-locked');
        showRemoteStream(msg.conversationId, msg.selectedTags);
        enterRemoteStopMode();
        updateInputState();
        window.updateWikiQueryState?.();
    } else {
        remoteStreaming = false;
        closeMirrorStream();
        exitRemoteStopMode();
        window.updateWikiQueryState?.();
        if (!isStreaming) {
            document.body.classList.remove('chat-streaming');
            document.getElementById('new-chat-btn').disabled = false;
            document.querySelector('.chat-sidebar').classList.remove('streaming-locked');
        }
        // Reload conversation to get final state with sources
        if (msg.conversationId && msg.conversationId === currentConversationId) {
            loadConversation(msg.conversationId, { broadcast: false });
        }
        loadConversations();
        updateInputState();
    }
};

// Refresh tags when navigating back to chat (covers create/edit/delete done on other pages)
document.addEventListener('spa:pageshow', async (e) => {
    if (e.detail.page === 'chat') {
        await loadTags();
        if (isWelcomeState) {
            renderWelcomeTags();
        }
        renderActiveTagBadges();
    }
});

// Cross-tab sync for non-streaming actions
window.syncChannel.onmessage = (e) => {
    const { type, payload } = e.data;

    if (type === 'new-chat') {
        if (document.body.dataset.activePage !== 'chat') return;
        if (isStreaming || remoteStreaming) return;
        newChat({ broadcast: false });
    } else if (type === 'load-conversation') {
        if (document.body.dataset.activePage !== 'chat') return;
        if (isStreaming || remoteStreaming) return;
        loadConversation(payload.conversationId, { broadcast: false });
    } else if (type === 'tags-changed') {
        refreshChatTags();
    } else if (type === 'conversations-changed') {
        if (payload.trashedId && payload.trashedId === currentConversationId) {
            newChat({ broadcast: false });
        }
        loadConversations();
    }
};

function closeMirrorStream() {
    if (mirrorEventSource) {
        mirrorEventSource.close();
        mirrorEventSource = null;
    }
}

let syncInFlight = false;
async function syncStreamingState() {
    if (isStreaming || syncInFlight) return;
    syncInFlight = true;
    try {
        const status = await apiGet('/api/chat/streaming');
        if (status.streaming && !remoteStreaming) {
            // Another tab started streaming — enter mirror mode
            remoteStreaming = true;
            document.body.classList.add('chat-streaming');
            document.getElementById('new-chat-btn').disabled = true;
            document.querySelector('.chat-sidebar').classList.add('streaming-locked');
            if (status.conversation_id) {
                showRemoteStream(status.conversation_id);
            }
            enterRemoteStopMode();
            updateInputState();
        } else if (!status.streaming && remoteStreaming) {
            // Streaming ended — exit mirror mode
            remoteStreaming = false;
            closeMirrorStream();
            exitRemoteStopMode();
            document.body.classList.remove('chat-streaming');
            document.getElementById('new-chat-btn').disabled = false;
            document.querySelector('.chat-sidebar').classList.remove('streaming-locked');
            if (currentConversationId) {
                loadConversation(currentConversationId, { broadcast: false });
            }
            loadConversations();
            updateInputState();
        }
    } catch {}
    syncInFlight = false;
}

function enterRemoteStopMode() {
    const sendBtn = document.getElementById('send-btn');
    sendBtn.textContent = 'Stop';
    sendBtn.type = 'button';
    sendBtn.classList.add('stop-mode');
    sendBtn.disabled = false;
    if (remoteStopHandler) {
        sendBtn.removeEventListener('click', remoteStopHandler);
    }
    remoteStopHandler = () => {
        streamingChannel.postMessage({ type: 'stop-request' });
        apiPost('/api/chat/stop');
    };
    sendBtn.addEventListener('click', remoteStopHandler);
}

function exitRemoteStopMode() {
    const sendBtn = document.getElementById('send-btn');
    if (remoteStopHandler) {
        sendBtn.removeEventListener('click', remoteStopHandler);
        remoteStopHandler = null;
    }
    sendBtn.textContent = 'Send';
    sendBtn.type = 'submit';
    sendBtn.classList.remove('stop-mode');
}

async function showRemoteStream(conversationId, broadcastTags) {
    // Resolve conversation ID from server if not provided
    if (!conversationId) {
        try {
            const status = await apiGet('/api/chat/streaming');
            conversationId = status.conversation_id;
        } catch {}
    }
    if (!conversationId) return;

    // Load the conversation (user message is already saved server-side)
    const data = await apiGet(`/api/conversations/${conversationId}`);

    currentConversationId = conversationId;
    const messagesEl = document.getElementById('chat-messages');
    messagesEl.innerHTML = '';

    exitWelcomeState();

    // Use tags from broadcast if available, otherwise fall back to conversation data
    if (broadcastTags && broadcastTags.length) {
        selectedTags = new Set(broadcastTags);
    } else {
        selectedTags = new Set(data.selected_tags || []);
    }
    renderActiveTagBadges();

    data.messages.forEach(msg => {
        appendMessage(msg.role, msg.content, msg.sources);
    });

    // Add the streaming assistant bubble
    const assistantDiv = appendMessage('assistant', '', null, true);
    const contentEl = assistantDiv.querySelector('.message-content');
    let fullText = '';

    // Connect to the SSE mirror endpoint
    closeMirrorStream();
    mirrorEventSource = new EventSource('/api/chat/stream-mirror');

    mirrorEventSource.addEventListener('token', (e) => {
        try {
            const data = JSON.parse(e.data);
            if (data.token !== undefined) {
                fullText += data.token;
                contentEl.innerHTML = renderContent(fullText);
                scrollToBottom();
            }
        } catch {}
    });

    mirrorEventSource.addEventListener('done', (e) => {
        closeMirrorStream();
        remoteStreaming = false;
        exitRemoteStopMode();
        document.body.classList.remove('chat-streaming');
        document.getElementById('new-chat-btn').disabled = false;
        document.querySelector('.chat-sidebar').classList.remove('streaming-locked');
        updateInputState();
        // Reload conversation for final state with sources
        if (currentConversationId) {
            loadConversation(currentConversationId, { broadcast: false });
        }
        loadConversations();
    });

    mirrorEventSource.addEventListener('error', () => {
        closeMirrorStream();
        remoteStreaming = false;
        exitRemoteStopMode();
        document.body.classList.remove('chat-streaming');
        document.getElementById('new-chat-btn').disabled = false;
        document.querySelector('.chat-sidebar').classList.remove('streaming-locked');
        updateInputState();
    });

    scrollToBottom();
    await loadConversations();
}

document.addEventListener('DOMContentLoaded', async () => {
    // Load tags first so they're available for any streaming mirror
    await loadTags();

    // Check if another tab/session is already streaming
    try {
        const status = await apiGet('/api/chat/streaming');
        if (status.streaming) {
            remoteStreaming = true;
            document.body.classList.add('chat-streaming');
            document.getElementById('new-chat-btn').disabled = true;
            document.querySelector('.chat-sidebar').classList.add('streaming-locked');
            if (status.conversation_id) {
                await showRemoteStream(status.conversation_id);
            }
            enterRemoteStopMode();
            updateInputState();
        }
    } catch {}

    await loadConversations();

    document.getElementById('chat-form').addEventListener('submit', handleSubmit);
    document.getElementById('new-chat-btn').addEventListener('click', newChat);

    // Trash button
    document.getElementById('trash-btn').addEventListener('click', openTrashView);
    document.getElementById('trash-back-btn').addEventListener('click', closeTrashView);

    // Trash confirm dialog
    const trashDialog = document.getElementById('trash-confirm-dialog');
    trashDialog.querySelector('.dialog-cancel').addEventListener('click', () => trashDialog.close());
    trashDialog.querySelector('.dialog-confirm').addEventListener('click', confirmTrash);

    // Permanent delete dialog
    const permDialog = document.getElementById('permanent-delete-dialog');
    permDialog.querySelector('.dialog-cancel').addEventListener('click', () => permDialog.close());
    permDialog.querySelector('.dialog-confirm').addEventListener('click', confirmPermanentDelete);

    // Depth settings dialog
    const depthDialog = document.getElementById('depth-settings-dialog');
    const depthOptionsEl = document.getElementById('depth-options');
    renderDepthOptions(depthOptionsEl, depthDialog);
    updateDepthIndicator();
    document.getElementById('depth-settings-btn').addEventListener('click', () => {
        depthDialog.showModal();
    });
    depthDialog.addEventListener('click', (e) => {
        if (e.target === depthDialog) depthDialog.close();
    });

    // Input validation — respects welcome state
    const chatInput = document.getElementById('chat-input');
    const sendBtn = document.getElementById('send-btn');
    chatInput.addEventListener('input', () => {
        updateInputState();
    });

    // Enter to send, Shift+Enter for newline
    chatInput.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            if (!isStreaming && !remoteStreaming) handleSubmit(e);
        }
    });

    // Initial welcome state — skip if mirroring a remote stream
    if (!remoteStreaming) enterWelcomeState();

    // Poll streaming state every second to keep all tabs in sync
    setInterval(syncStreamingState, 1000);
});

// ─── Welcome State Management ───

function enterWelcomeState() {
    isWelcomeState = true;

    // Hide the small tag-selector row
    document.getElementById('tag-selector').style.display = 'none';

    // Clear selected tags
    selectedTags.clear();

    // Render welcome tags
    renderWelcomeTags();

    // Disable input
    updateInputState();
}

function exitWelcomeState() {
    isWelcomeState = false;

    // Show the small tag-selector row
    document.getElementById('tag-selector').style.display = '';

    // Render read-only active tag badges
    renderActiveTagBadges();

    // Enable input
    updateInputState();
}

function renderWelcomeTags() {
    const container = document.getElementById('welcome-tags');
    if (!container) return;
    container.innerHTML = '';

    // If no tags configured, skip enforcement
    if (!allTags.length) {
        exitWelcomeState();
        return;
    }

    const sorted = [...allTags].sort((a, b) => (b.file_count || 0) - (a.file_count || 0));
    sorted.forEach(tag => {
        const pill = document.createElement('span');
        pill.className = 'welcome-tag';
        pill.textContent = tag.display_name;
        pill.style.borderColor = tag.color;
        pill.style.color = tag.color;
        pill.style.backgroundColor = 'transparent';
        pill.dataset.tagName = tag.name;

        pill.addEventListener('click', () => {
            toggleWelcomeTag(tag, pill);
        });

        container.appendChild(pill);
    });
}

function toggleWelcomeTag(tag, pill) {
    if (selectedTags.has(tag.name)) {
        selectedTags.delete(tag.name);
        pill.classList.remove('selected');
        pill.style.color = tag.color;
        pill.style.backgroundColor = 'transparent';
    } else {
        selectedTags.add(tag.name);
        pill.classList.add('selected');
        pill.style.color = '#fff';
        pill.style.backgroundColor = tag.color;
    }

    // Update input enabled/disabled
    updateInputState();
}


function updateDepthIndicator() {
    const el = document.getElementById('depth-indicator');
    if (!el) return;
    const level = DEPTH_LEVELS.find(l => l.key === currentDepth) || DEPTH_LEVELS[1];
    el.textContent = '\u2699 ' + level.label;
}

function renderDepthOptions(container, dialog) {
    container.innerHTML = '';
    DEPTH_LEVELS.forEach(level => {
        const card = document.createElement('div');
        card.className = 'depth-option' + (level.key === currentDepth ? ' active' : '');
        card.innerHTML = `
            <div class="depth-option-label">${level.label}</div>
            <div class="depth-option-desc">${level.desc}</div>
        `;
        card.addEventListener('click', () => {
            currentDepth = level.key;
            localStorage.setItem('georag-depth', currentDepth);
            container.querySelectorAll('.depth-option').forEach(el => el.classList.remove('active'));
            card.classList.add('active');
            updateDepthIndicator();
            dialog.close();
        });
        container.appendChild(card);
    });
}

function getDepthParams() {
    const level = DEPTH_LEVELS.find(l => l.key === currentDepth) || DEPTH_LEVELS[1];
    return { top_k_per_tag: level.topK, max_context_chunks: level.maxCtx };
}

function updateInputState() {
    const chatInput = document.getElementById('chat-input');
    const sendBtn = document.getElementById('send-btn');
    const depthBtn = document.getElementById('depth-settings-btn');

    if (isStreaming) {
        chatInput.disabled = true;
        sendBtn.disabled = false;
        if (depthBtn) depthBtn.disabled = true;
        return;
    }

    if (remoteStreaming) {
        chatInput.disabled = true;
        chatInput.placeholder = 'Chat is active in another tab...';
        // Stop button is managed by enterRemoteStopMode — don't override it
        sendBtn.disabled = false;
        if (depthBtn) depthBtn.disabled = true;
        return;
    }

    if (depthBtn) depthBtn.disabled = false;

    if (window.libraryIsProcessing) {
        chatInput.disabled = true;
        sendBtn.disabled = true;
        chatInput.placeholder = 'Chat is unavailable while the library is processing...';
        return;
    }

    const hasText = chatInput.value.trim().length > 0;

    if (isWelcomeState && allTags.length > 0 && selectedTags.size === 0) {
        // Disabled: no tags selected in welcome state
        chatInput.disabled = true;
        sendBtn.disabled = true;
        chatInput.placeholder = 'Select at least one topic above to start...';
    } else {
        // Enabled
        chatInput.disabled = false;
        chatInput.placeholder = 'Ask a geotechnical engineering question...';
        sendBtn.disabled = !hasText;
    }
}

// Expose streaming state for other scripts (e.g. panel.js)
window.chatIsStreaming = () => isStreaming || remoteStreaming;
// Allow panel.js to trigger input state refresh when processing starts/stops
window.updateChatInputState = updateInputState;

// ─── Tags ───

async function loadTags() {
    allTags = await apiGet('/api/tags');
}

async function refreshChatTags() {
    await loadTags();
    // Remove stale selections (deleted tags)
    const validNames = new Set(allTags.map(t => t.name));
    for (const name of selectedTags) {
        if (!validNames.has(name)) selectedTags.delete(name);
    }
    if (isWelcomeState) {
        renderWelcomeTags();
    }
    renderActiveTagBadges();
}
window.refreshChatTags = refreshChatTags;

function renderActiveTagBadges() {
    const container = document.getElementById('tag-badges');
    container.innerHTML = '';

    allTags
        .filter(tag => selectedTags.has(tag.name))
        .forEach(tag => {
            container.appendChild(createTagBadge(tag, true, null));
        });
}

// ─── Conversations ───

async function loadConversations() {
    const conversations = await apiGet('/api/conversations');
    const list = document.getElementById('conversation-list');

    if (!conversations.length) {
        list.innerHTML = '<p class="secondary" style="font-size:0.85rem;padding:0.5rem;">No conversations yet</p>';
        updateTrashCount();
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
                promptTrashConversation(conv.id);
            } else {
                loadConversation(conv.id);
            }
        });
        list.appendChild(item);
    });

    updateTrashCount();
}

async function loadConversation(convId, { broadcast = true } = {}) {
    if (isStreaming) return;
    currentConversationId = convId;
    const data = await apiGet(`/api/conversations/${convId}`);
    const messagesEl = document.getElementById('chat-messages');
    messagesEl.innerHTML = '';

    // Exit welcome state
    exitWelcomeState();

    // Restore selected tags
    selectedTags = new Set(data.selected_tags || []);
    renderActiveTagBadges();

    data.messages.forEach(msg => {
        appendMessage(msg.role, msg.content, msg.sources);
    });

    await loadConversations();
    scrollToBottom();

    if (broadcast) {
        window.syncChannel.postMessage({ type: 'load-conversation', payload: { conversationId: convId } });
    }
}

// ─── Trash (Soft Delete) ───

function promptTrashConversation(convId) {
    if (isStreaming) return;
    pendingDeleteConvId = convId;
    document.getElementById('trash-confirm-dialog').showModal();
}

async function confirmTrash() {
    document.getElementById('trash-confirm-dialog').close();
    if (!pendingDeleteConvId) return;

    const convId = pendingDeleteConvId;
    pendingDeleteConvId = null;

    await apiDelete(`/api/conversations/${convId}`);
    if (convId === currentConversationId) {
        newChat({ broadcast: false });
    }
    await loadConversations();
    window.syncChannel.postMessage({ type: 'conversations-changed', payload: { trashedId: convId } });
}

async function openTrashView() {
    isTrashViewOpen = true;
    document.getElementById('new-chat-btn').style.display = 'none';
    document.getElementById('conversation-list').style.display = 'none';
    document.getElementById('trash-btn').style.display = 'none';
    document.getElementById('trash-view').style.display = '';
    await loadTrashList();
}

function closeTrashView() {
    isTrashViewOpen = false;
    document.getElementById('new-chat-btn').style.display = '';
    document.getElementById('conversation-list').style.display = '';
    document.getElementById('trash-btn').style.display = '';
    document.getElementById('trash-view').style.display = 'none';
}

async function loadTrashList() {
    const items = await apiGet('/api/conversations/trash');
    const list = document.getElementById('trash-list');

    if (!items.length) {
        list.innerHTML = '<p class="secondary" style="font-size:0.85rem;padding:0.5rem;">Trash is empty</p>';
        return;
    }

    list.innerHTML = '';
    items.forEach(conv => {
        const item = document.createElement('div');
        item.className = 'conversation-item trash-item';
        item.innerHTML = `
            <span class="conv-title">${escapeHtml(conv.title)}</span>
            <span class="conv-date">Deleted ${formatDate(conv.deleted_at)}</span>
            <span class="trash-actions">
                <span class="trash-restore" title="Restore">&#x21A9;</span>
                <span class="trash-permanent-delete" title="Delete permanently">&#x2715;</span>
            </span>
        `;
        item.querySelector('.trash-restore').addEventListener('click', (e) => {
            e.stopPropagation();
            restoreConversation(conv.id);
        });
        item.querySelector('.trash-permanent-delete').addEventListener('click', (e) => {
            e.stopPropagation();
            promptPermanentDelete(conv.id);
        });
        list.appendChild(item);
    });
}

async function restoreConversation(convId) {
    await apiPost(`/api/conversations/${convId}/restore`);
    await loadTrashList();
    await loadConversations();
    window.syncChannel.postMessage({ type: 'conversations-changed', payload: {} });
}

function promptPermanentDelete(convId) {
    pendingDeleteConvId = convId;
    document.getElementById('permanent-delete-dialog').showModal();
}

async function confirmPermanentDelete() {
    document.getElementById('permanent-delete-dialog').close();
    if (!pendingDeleteConvId) return;

    const convId = pendingDeleteConvId;
    pendingDeleteConvId = null;

    await apiDelete(`/api/conversations/${convId}/permanent`);
    await loadTrashList();
    await updateTrashCount();
    window.syncChannel.postMessage({ type: 'conversations-changed', payload: { trashedId: convId } });
}

async function updateTrashCount() {
    const items = await apiGet('/api/conversations/trash');
    const btn = document.getElementById('trash-btn');
    if (items.length > 0) {
        btn.textContent = `\u{1F5D1} Trash (${items.length})`;
    } else {
        btn.textContent = '\u{1F5D1} Trash';
    }
}

function newChat({ broadcast = true } = {}) {
    if (isStreaming) return;
    currentConversationId = null;
    document.getElementById('chat-messages').innerHTML = `
        <div class="chat-welcome" id="chat-welcome">
            <p class="secondary">Select topics to focus your search:</p>
            <div id="welcome-tags" class="welcome-tags"></div>
        </div>
    `;

    // Re-enter welcome state
    enterWelcomeState();

    // Close trash view if open
    if (isTrashViewOpen) closeTrashView();

    loadConversations();

    if (broadcast) {
        window.syncChannel.postMessage({ type: 'new-chat', payload: {} });
    }
}

// ─── Chat ───

async function handleSubmit(e) {
    e.preventDefault();
    const input = document.getElementById('chat-input');
    const message = input.value.trim();
    if (!message || isStreaming || remoteStreaming || window.libraryIsProcessing) return;

    // Check LM Studio before sending
    if (!(await window.requireLmStudio())) return;

    // Block submit if welcome state and no tags selected
    if (isWelcomeState && allTags.length > 0 && selectedTags.size === 0) return;

    // Clear welcome message and exit welcome state
    const welcome = document.querySelector('.chat-welcome');
    if (welcome) welcome.remove();
    if (isWelcomeState) {
        exitWelcomeState();
    }

    // Show user message
    appendMessage('user', message);
    input.value = '';

    // Start streaming
    isStreaming = true;
    document.body.classList.add('chat-streaming');
    window.updateWikiQueryState?.();
    const sendBtn = document.getElementById('send-btn');
    sendBtn.textContent = 'Stop';
    sendBtn.type = 'button';
    sendBtn.classList.add('stop-mode');
    document.getElementById('new-chat-btn').disabled = true;
    document.querySelector('.chat-sidebar').classList.add('streaming-locked');
    updateInputState();

    const controller = new AbortController();
    activeAbortController = controller;
    const stopHandler = () => {
        controller.abort();
        apiPost('/api/chat/stop');
    };
    sendBtn.addEventListener('click', stopHandler, { once: true });

    const assistantDiv = appendMessage('assistant', '', null, true);
    const contentEl = assistantDiv.querySelector('.message-content');
    let broadcastedStart = false;

    try {
        const depthParams = getDepthParams();
        const response = await fetch('/api/chat', {
            method: 'POST',
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({
                message: message,
                conversation_id: currentConversationId,
                tag_names: Array.from(selectedTags),
                top_k_per_tag: depthParams.top_k_per_tag,
                max_context_chunks: depthParams.max_context_chunks,
            }),
            signal: controller.signal,
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

                        if (data.error) {
                            contentEl.textContent = 'Error: ' + data.error;
                            // Check if it's an LM Studio connectivity issue
                            window.requireLmStudio();
                        } else if (data.token !== undefined) {
                            // Broadcast streaming status on first token
                            if (!broadcastedStart) {
                                broadcastedStart = true;
                                streamingChannel.postMessage({ streaming: true, conversationId: currentConversationId, selectedTags: Array.from(selectedTags) });
                            }
                            fullText += data.token;
                            contentEl.innerHTML = renderContent(fullText);
                            scrollToBottom();
                        }
                    } catch {}
                }
                if (line.startsWith('data: ') && line.includes('"conversation_id"')) {
                    try {
                        const doneData = JSON.parse(line.slice(6));
                        if (doneData.conversation_id) {
                            currentConversationId = doneData.conversation_id;
                            if (!broadcastedStart) {
                                broadcastedStart = true;
                                streamingChannel.postMessage({ streaming: true, conversationId: currentConversationId, selectedTags: Array.from(selectedTags) });
                            }
                        }
                        if (doneData.sources && doneData.sources.length) {
                            appendSources(assistantDiv, doneData.sources);
                        }
                        // Wiki/RAG source indicator
                        if (doneData.context_source && doneData.context_source !== 'none') {
                            appendContextBadge(assistantDiv, doneData.context_source, doneData.wiki_pages_used);
                        }
                    } catch {}
                }
            }
        }

    } catch (err) {
        if (err.name !== 'AbortError') {
            contentEl.textContent = 'Error: ' + err.message;
            window.requireLmStudio();
        }
    }

    sendBtn.removeEventListener('click', stopHandler);
    activeAbortController = null;
    sendBtn.textContent = 'Send';
    sendBtn.type = 'submit';
    sendBtn.classList.remove('stop-mode');
    isStreaming = false;
    window.updateWikiQueryState?.();
    streamingChannel.postMessage({ streaming: false, conversationId: currentConversationId });
    if (!remoteStreaming) document.body.classList.remove('chat-streaming');
    document.getElementById('new-chat-btn').disabled = false;
    document.querySelector('.chat-sidebar').classList.remove('streaming-locked');
    updateInputState();
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
        contentEl.innerHTML = renderContent(content);
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

    const sourcesEl = document.createElement('div');
    sourcesEl.className = 'message-sources';
    sourcesEl.innerHTML = `<div class="sources-header">Sources (${sources.length})</div>`;

    sources.forEach((src, idx) => {
        const item = document.createElement('div');
        item.className = 'source-item';

        const link = document.createElement('a');
        link.className = 'source-link';
        link.href = '#';
        const page = src.page ? ` (p.${src.page})` : '';
        const score = src.score ? ` [${(src.score * 100).toFixed(0)}% relevant]` : '';
        link.textContent = `[${idx + 1}] ${src.filename || src.file_path}${page}${score}`;
        link.addEventListener('click', (e) => {
            e.preventDefault();
            apiPost('/api/documents/open-by-path', { file_path: src.file_path });
        });

        item.appendChild(link);
        sourcesEl.appendChild(item);
    });

    messageDiv.appendChild(sourcesEl);
}

function appendContextBadge(messageDiv, contextSource, wikiPagesUsed) {
    const badgeEl = document.createElement('span');
    badgeEl.className = `chat-source-badge ${contextSource}`;
    const labels = { wiki: 'Wiki', rag: 'Documents', hybrid: 'Wiki + Docs' };
    badgeEl.textContent = labels[contextSource] || contextSource;

    // Insert badge into the message header area
    const contentEl = messageDiv.querySelector('.message-content');
    if (contentEl) {
        contentEl.insertAdjacentElement('afterend', badgeEl);
    }

    // Show wiki page links if applicable
    if (wikiPagesUsed && wikiPagesUsed.length > 0) {
        const wikiLinksEl = document.createElement('div');
        wikiLinksEl.className = 'chat-wiki-pages';
        wikiLinksEl.innerHTML = 'Wiki: ' + wikiPagesUsed.map(p =>
            `<a href="/wiki" data-slug="${escapeHtml(p.slug)}" title="${escapeHtml(p.title)}">${escapeHtml(p.title)}</a>`
        ).join(', ');
        wikiLinksEl.querySelectorAll('a').forEach(a => {
            a.addEventListener('click', (e) => {
                e.preventDefault();
                // Navigate to wiki page
                window.history.pushState({}, '', '/wiki');
                window.dispatchEvent(new PopStateEvent('popstate'));
                // After navigation, try to open the specific page
                setTimeout(() => {
                    document.dispatchEvent(new CustomEvent('wiki:navigate', { detail: { slug: a.dataset.slug } }));
                }, 100);
            });
        });
        messageDiv.appendChild(wikiLinksEl);
    }
}

function scrollToBottom() {
    const el = document.getElementById('chat-messages');
    el.scrollTop = el.scrollHeight;
}
