// geoRAG Chat Interface

let currentConversationId = null;
let selectedTags = new Set();
let allTags = [];
let isStreaming = false;
let isWelcomeState = true;

document.addEventListener('DOMContentLoaded', async () => {
    await loadTags();
    await loadConversations();

    document.getElementById('chat-form').addEventListener('submit', handleSubmit);
    document.getElementById('new-chat-btn').addEventListener('click', newChat);

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
            if (!isStreaming) handleSubmit(e);
        }
    });

    // Initial welcome state
    enterWelcomeState();
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

    const sorted = [...allTags].sort((a, b) => a.display_name.length - b.display_name.length);
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


function updateInputState() {
    const chatInput = document.getElementById('chat-input');
    const sendBtn = document.getElementById('send-btn');

    if (isStreaming) {
        chatInput.disabled = true;
        sendBtn.disabled = false;
        return;
    }

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
window.chatIsStreaming = () => isStreaming;
// Allow panel.js to trigger input state refresh when processing starts/stops
window.updateChatInputState = updateInputState;

// ─── Tags ───

async function loadTags() {
    allTags = await apiGet('/api/tags');
}

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
        <div class="chat-welcome" id="chat-welcome">
            <h2>geoRAG</h2>
            <p>Geotechnical Engineering RAG Assistant</p>
            <p class="secondary">Select topics to focus your search</p>
            <div id="welcome-tags" class="welcome-tags"></div>
        </div>
    `;

    // Re-enter welcome state
    enterWelcomeState();

    loadConversations();
}

// ─── Chat ───

async function handleSubmit(e) {
    e.preventDefault();
    const input = document.getElementById('chat-input');
    const message = input.value.trim();
    if (!message || isStreaming || window.libraryIsProcessing) return;

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
    const sendBtn = document.getElementById('send-btn');
    sendBtn.textContent = 'Stop';
    sendBtn.type = 'button';
    sendBtn.classList.add('stop-mode');
    updateInputState();

    const controller = new AbortController();
    const stopHandler = () => { controller.abort(); };
    sendBtn.addEventListener('click', stopHandler, { once: true });

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
        if (err.name !== 'AbortError') {
            contentEl.textContent = 'Error: ' + err.message;
        }
    }

    sendBtn.removeEventListener('click', stopHandler);
    sendBtn.textContent = 'Send';
    sendBtn.type = 'submit';
    sendBtn.classList.remove('stop-mode');
    isStreaming = false;
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

    const sourcesEl = document.createElement('div');
    sourcesEl.className = 'message-sources';
    sourcesEl.innerHTML = `<div class="sources-header">Sources (${sources.length})</div>`;

    sources.forEach(src => {
        const item = document.createElement('div');
        item.className = 'source-item';

        const link = document.createElement('a');
        link.className = 'source-link';
        link.href = '#';
        const page = src.page ? ` (p.${src.page})` : '';
        const score = src.score ? ` [${(src.score * 100).toFixed(0)}%]` : '';
        link.textContent = `${src.filename || src.file_path}${page}${score}`;
        link.addEventListener('click', (e) => {
            e.preventDefault();
            apiPost('/api/documents/open-by-path', { file_path: src.file_path });
        });

        item.appendChild(link);
        sourcesEl.appendChild(item);
    });

    messageDiv.appendChild(sourcesEl);
}

function scrollToBottom() {
    const el = document.getElementById('chat-messages');
    el.scrollTop = el.scrollHeight;
}
