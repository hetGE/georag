// Utility functions for geoRAG

function formatBytes(bytes) {
    if (!bytes || bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

function formatDate(dateStr) {
    if (!dateStr) return '';
    const d = new Date(dateStr);
    return d.toLocaleDateString() + ' ' + d.toLocaleTimeString([], {hour: '2-digit', minute: '2-digit'});
}

function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

async function apiGet(url) {
    const res = await fetch(url);
    return res.json();
}

async function apiPost(url, data = {}) {
    const res = await fetch(url, {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify(data),
    });
    return res.json();
}

async function apiDelete(url, data = null) {
    const options = {method: 'DELETE'};
    if (data) {
        options.headers = {'Content-Type': 'application/json'};
        options.body = JSON.stringify(data);
    }
    const res = await fetch(url, options);
    return res.json();
}

function createTagBadge(tag, selected = false, onClick = null) {
    const badge = document.createElement('span');
    badge.className = 'tag-badge' + (selected ? ' selected' : '');
    badge.textContent = tag.display_name;
    badge.style.borderColor = tag.color;
    badge.style.color = selected ? '#fff' : tag.color;
    badge.style.backgroundColor = selected ? tag.color : 'transparent';
    badge.dataset.tagName = tag.name;

    if (onClick) {
        badge.addEventListener('click', () => onClick(tag, badge));
    }

    return badge;
}
