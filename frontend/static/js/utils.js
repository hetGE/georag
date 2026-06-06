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

async function apiPut(url, data = {}) {
    const res = await fetch(url, {
        method: 'PUT',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify(data),
    });
    return res.json();
}

// Set a button into a busy/pending state, returning a closure that restores
// its prior textContent / disabled / aria-busy exactly. Pico renders the
// spinner from aria-busy. Pass label=null to keep the existing text.
function setBtnBusy(btn, label) {
    if (!btn) return () => {};
    const prevText = btn.textContent;
    const prevDisabled = btn.disabled;
    const prevBusy = btn.getAttribute('aria-busy');
    btn.disabled = true;
    btn.setAttribute('aria-busy', 'true');
    if (label != null) btn.textContent = label;
    return () => {
        btn.disabled = prevDisabled;
        if (prevBusy === null) {
            btn.removeAttribute('aria-busy');
        } else {
            btn.setAttribute('aria-busy', prevBusy);
        }
        if (label != null) btn.textContent = prevText;
    };
}

// The 12-spoke green spinner markup, shared so loaders can be injected anywhere
// without copy-pasting the SVG (mirrors the inline one in spa.html).
function spinnerWheelHTML() {
    let spokes = '';
    for (let i = 0; i < 12; i++) {
        spokes += `<rect class="spoke" x="46" y="5" width="8" height="22" rx="4" transform="rotate(${i * 30} 50 50)"/>`;
    }
    return `<svg class="spinner-wheel" viewBox="0 0 100 100">${spokes}</svg>`;
}

// Inject a centered spinner overlay into any container. Returns a restore()
// closure that removes the overlay and resets the container's position.
// If the caller later replaces the container's innerHTML, the overlay is
// auto-discarded and restore() is a harmless no-op.
function showLoader(container, message) {
    const el = typeof container === 'string' ? document.querySelector(container) : container;
    if (!el) return () => {};
    const overlay = document.createElement('div');
    overlay.className = 'loading-overlay';
    overlay.innerHTML = spinnerWheelHTML() + (message ? `<p>${escapeHtml(message)}</p>` : '');
    const hadInlinePosition = !!el.style.position;
    if (getComputedStyle(el).position === 'static') el.style.position = 'relative';
    el.appendChild(overlay);
    return () => {
        overlay.remove();
        if (!hadInlinePosition) el.style.position = '';
    };
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

// Cross-tab sync channel (non-streaming actions)
window.syncChannel = new BroadcastChannel('app-sync');
