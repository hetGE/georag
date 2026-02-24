// GeoRAG Documents Interface

let currentPage = 1;
let allTagsList = [];
let selectedFileIds = new Set();
let debounceTimer = null;

document.addEventListener('DOMContentLoaded', async () => {
    await loadTagFilter();
    await loadStats();
    loadDocuments();

    document.getElementById('search-input').addEventListener('input', () => {
        clearTimeout(debounceTimer);
        debounceTimer = setTimeout(() => { currentPage = 1; loadDocuments(); }, 300);
    });

    document.getElementById('ext-filter').addEventListener('change', () => { currentPage = 1; loadDocuments(); });
    document.getElementById('tag-filter').addEventListener('change', () => { currentPage = 1; loadDocuments(); });
    document.getElementById('status-filter').addEventListener('change', () => { currentPage = 1; loadDocuments(); });

    document.getElementById('scan-btn').addEventListener('click', scanFiles);
    document.getElementById('process-btn').addEventListener('click', processSelected);
    document.getElementById('autotag-btn').addEventListener('click', autotagSelected);
    document.getElementById('process-new-btn').addEventListener('click', processNewFiles);
    document.getElementById('select-all').addEventListener('change', toggleSelectAll);

    // Close modal
    document.querySelector('.close-modal')?.addEventListener('click', () => {
        document.getElementById('tag-modal').close();
    });
});

async function loadTagFilter() {
    allTagsList = await apiGet('/api/tags');
    const select = document.getElementById('tag-filter');
    allTagsList.forEach(tag => {
        const opt = document.createElement('option');
        opt.value = tag.name;
        opt.textContent = `${tag.display_name} (${tag.file_count})`;
        select.appendChild(opt);
    });
}

async function loadStats() {
    const stats = await apiGet('/api/documents/stats');
    document.getElementById('stats-total').textContent = `${stats.total.toLocaleString()} files`;

    const statusParts = Object.entries(stats.by_status || {})
        .map(([k, v]) => `${v} ${k}`)
        .join(', ');
    document.getElementById('stats-status').textContent = statusParts || 'No files scanned';
}

async function loadDocuments() {
    const search = document.getElementById('search-input').value;
    const ext = document.getElementById('ext-filter').value;
    const tag = document.getElementById('tag-filter').value;
    const status = document.getElementById('status-filter').value;

    const params = new URLSearchParams({page: currentPage, per_page: 50});
    if (search) params.set('search', search);
    if (ext) params.set('extension', ext);
    if (tag) params.set('tag', tag);
    if (status) params.set('status', status);

    const data = await apiGet(`/api/documents?${params}`);
    renderFileTable(data.files);
    renderPagination(data);
}

function renderFileTable(files) {
    const tbody = document.getElementById('file-list');

    if (!files.length) {
        tbody.innerHTML = '<tr><td colspan="8" class="center">No files found</td></tr>';
        return;
    }

    tbody.innerHTML = files.map(f => {
        const checked = selectedFileIds.has(f.id) ? 'checked' : '';
        const tags = (f.tags || []).map(t =>
            `<span class="tag-mini" style="background:${t.color}">${t.display_name}</span>`
        ).join('');

        return `<tr>
            <td><input type="checkbox" class="file-cb" data-id="${f.id}" ${checked}></td>
            <td title="${escapeHtml(f.relative_path)}" onclick="openTagModal(${f.id}, '${escapeHtml(f.filename)}')">${escapeHtml(f.filename)}</td>
            <td>${f.extension || '-'}</td>
            <td>${formatBytes(f.size_bytes)}</td>
            <td title="${escapeHtml(f.parent_directory || '')}">${truncatePath(f.parent_directory)}</td>
            <td><span class="status-badge status-${f.scan_status}">${f.scan_status}</span></td>
            <td class="tag-cell">${tags || '-'}</td>
            <td>${f.chunk_count || 0}</td>
        </tr>`;
    }).join('');

    // Re-bind checkboxes
    document.querySelectorAll('.file-cb').forEach(cb => {
        cb.addEventListener('change', (e) => {
            const id = parseInt(e.target.dataset.id);
            if (e.target.checked) {
                selectedFileIds.add(id);
            } else {
                selectedFileIds.delete(id);
            }
        });
    });
}

function truncatePath(path) {
    if (!path) return '-';
    const parts = path.split('/');
    if (parts.length <= 3) return path;
    return '.../' + parts.slice(-2).join('/');
}

function renderPagination(data) {
    const el = document.getElementById('pagination');
    if (data.pages <= 1) {
        el.innerHTML = '';
        return;
    }

    el.innerHTML = `
        <button onclick="goPage(${data.page - 1})" ${data.page <= 1 ? 'disabled' : ''} class="outline">&laquo; Prev</button>
        <span class="page-info">Page ${data.page} of ${data.pages} (${data.total.toLocaleString()} files)</span>
        <button onclick="goPage(${data.page + 1})" ${data.page >= data.pages ? 'disabled' : ''} class="outline">Next &raquo;</button>
    `;
}

function goPage(page) {
    currentPage = page;
    loadDocuments();
}

function toggleSelectAll(e) {
    const checked = e.target.checked;
    document.querySelectorAll('.file-cb').forEach(cb => {
        cb.checked = checked;
        const id = parseInt(cb.dataset.id);
        if (checked) selectedFileIds.add(id);
        else selectedFileIds.delete(id);
    });
}

async function scanFiles() {
    const btn = document.getElementById('scan-btn');
    btn.disabled = true;
    btn.textContent = 'Scanning...';
    btn.setAttribute('aria-busy', 'true');

    try {
        const result = await apiPost('/api/processing/scan');
        btn.textContent = `Found ${result.files_found?.toLocaleString() || 0} files`;
        await loadStats();
        loadDocuments();
    } catch (e) {
        btn.textContent = 'Scan Failed';
    }

    btn.setAttribute('aria-busy', 'false');
    setTimeout(() => {
        btn.disabled = false;
        btn.textContent = 'Scan Files';
    }, 2000);
}

async function processSelected() {
    const ids = Array.from(selectedFileIds);
    if (!ids.length) {
        alert('Select files to process first');
        return;
    }

    await apiPost('/api/processing/start', {file_ids: ids});
    // processing.js will handle progress polling
}

async function processNewFiles() {
    const btn = document.getElementById('process-new-btn');
    btn.disabled = true;
    btn.setAttribute('aria-busy', 'true');
    const result = await apiPost('/api/processing/start', {});
    btn.disabled = false;
    btn.setAttribute('aria-busy', 'false');
    if (result.error) {
        alert(result.error);
    }
}

async function autotagSelected() {
    const ids = Array.from(selectedFileIds);
    if (!ids.length) {
        alert('Select files to auto-tag first');
        return;
    }

    // Process with auto-tagging (no specific tags = auto-tag mode)
    await apiPost('/api/processing/start', {file_ids: ids});
}

async function openTagModal(fileId, filename) {
    const modal = document.getElementById('tag-modal');
    document.getElementById('tag-modal-filename').textContent = filename;

    const content = document.getElementById('tag-modal-content');
    content.innerHTML = '<p>Loading tags...</p>';
    modal.showModal();

    // Get file's current tags
    const fileData = await apiGet(`/api/documents?search=${encodeURIComponent(filename)}&per_page=1`);
    const file = fileData.files?.[0];
    const currentTags = new Set((file?.tags || []).map(t => t.name));

    content.innerHTML = `
        <div class="tag-modal-open-file">
            <a href="/api/documents/${fileId}/open" target="_blank" class="open-file-btn outline">
                Open File
            </a>
            <span class="open-file-path" title="${escapeHtml(file?.relative_path || '')}">${truncatePath(file?.parent_directory || '')}/${escapeHtml(filename)}</span>
        </div>
        <div class="tag-modal-list"></div>`;
    const list = content.querySelector('.tag-modal-list');

    allTagsList.forEach(tag => {
        const hasTag = currentTags.has(tag.name);
        const item = document.createElement('div');
        item.className = 'tag-modal-item';
        item.innerHTML = `
            <span>
                <span class="tag-mini" style="background:${tag.color}">${tag.display_name}</span>
                <small style="margin-left:0.5rem;color:var(--pico-muted-color)">${tag.description || ''}</small>
            </span>
            <button class="${hasTag ? 'secondary outline' : 'outline'}" data-tag="${tag.name}" data-has="${hasTag}">
                ${hasTag ? 'Remove' : 'Add'}
            </button>
        `;

        item.querySelector('button').addEventListener('click', async (e) => {
            const btn = e.target;
            const tagName = btn.dataset.tag;
            const has = btn.dataset.has === 'true';

            if (has) {
                await apiDelete(`/api/documents/${fileId}/tags/${tagName}`);
                btn.textContent = 'Add';
                btn.className = 'outline';
                btn.dataset.has = 'false';
            } else {
                await apiPost(`/api/documents/${fileId}/tags/${tagName}`, {});
                btn.textContent = 'Remove';
                btn.className = 'secondary outline';
                btn.dataset.has = 'true';
            }
            loadDocuments();
        });

        list.appendChild(item);
    });
}
