// geoRAG Chat Interface

let currentConversationId = null;
let selectedTags = new Set();
let allTags = [];
let isStreaming = false;

document.addEventListener('DOMContentLoaded', async () => {
    await loadTags();
    await loadConversations();

    document.getElementById('chat-form').addEventListener('submit', handleSubmit);
    document.getElementById('new-chat-btn').addEventListener('click', newChat);

    // Disable send button when input is empty
    const chatInput = document.getElementById('chat-input');
    const sendBtn = document.getElementById('send-btn');
    sendBtn.disabled = true;
    chatInput.addEventListener('input', () => {
        sendBtn.disabled = !chatInput.value.trim();
    });

    // Enter to send, Shift+Enter for newline
    chatInput.addEventListener('keydown', (e) => {
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
            <h2>geoRAG</h2>
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
    document.getElementById('send-btn').disabled = !document.getElementById('chat-input').value.trim();
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

