// geoRAG Documents Interface

let currentPage = 1;
let allTagsList = [];
let selectedFileIds = new Set();
let debounceTimer = null;
let lastLoadedFiles = [];
let documentsInitialized = false;

async function initDocuments() {
    if (documentsInitialized) return;
    documentsInitialized = true;

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

    document.getElementById('select-all').addEventListener('change', toggleSelectAll);
    document.getElementById('process-btn').addEventListener('click', markSelectedAsNew);
    document.getElementById('skip-btn').addEventListener('click', markSelectedAsSkipped);
document.getElementById('multi-tag-btn').addEventListener('click', openMultiTagModal);
    document.getElementById('manage-tags-btn').addEventListener('click', openTagAdminModal);

    // Show action buttons once processing status is known
    checkActionButtonVisibility();

    // Close modals
    document.querySelector('.close-modal')?.addEventListener('click', () => {
        document.getElementById('tag-modal').close();
    });
    document.querySelector('.close-multi-tag')?.addEventListener('click', () => {
        document.getElementById('multi-tag-modal').close();
    });
    document.querySelector('.close-tag-admin')?.addEventListener('click', () => {
        document.getElementById('tag-admin-modal').close();
    });
}

// Init immediately if documents page is already active
if (document.body.dataset.activePage === 'documents') {
    document.addEventListener('DOMContentLoaded', initDocuments);
}

// Lazy-init when navigating to documents via SPA router
document.addEventListener('spa:pageshow', (e) => {
    if (e.detail.page === 'documents') initDocuments();
});

async function loadTagFilter() {
    allTagsList = await apiGet('/api/tags');
    const select = document.getElementById('tag-filter');
    const noTagOpt = document.createElement('option');
    noTagOpt.value = '__none__';
    noTagOpt.textContent = 'No tag';
    select.appendChild(noTagOpt);
    allTagsList.forEach(tag => {
        const opt = document.createElement('option');
        opt.value = tag.name;
        opt.textContent = `${tag.display_name} (${tag.file_count})`;
        select.appendChild(opt);
    });
}

async function loadStats() {
    const stats = await apiGet('/api/documents/stats');
    document.getElementById('stats-total').textContent = `${stats.total} files`;

    const parts = [];
    if (stats.untagged > 0) parts.push(`${stats.untagged} untagged`);
    Object.entries(stats.by_status || {}).forEach(([k, v]) => parts.push(`${v} ${k}`));
    document.getElementById('stats-status').textContent = parts.join(', ') || 'No files scanned';
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
    lastLoadedFiles = data.files;
    renderFileTable(data.files);
    renderPagination(data);
    updateMultiTagVisibility();
}

function renderFileTable(files) {
    const tbody = document.getElementById('file-list');

    if (!files.length) {
        tbody.innerHTML = '<tr><td colspan="8" class="center">No files found</td></tr>';
        return;
    }

    tbody.innerHTML = files.map(f => {
        const checked = selectedFileIds.has(f.id) ? 'checked' : '';
        const tags = (f.tags || []).map(t => {
            const abbr = t.display_name.split(/\s+/).map(w => w[0]).join('').toUpperCase();
            return `<span class="tag-mini" style="background:${t.color}" title="${escapeHtml(t.display_name)}">${abbr}</span>`;
        }).join('');

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
            updateMultiTagVisibility();
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
    updateMultiTagVisibility();
}

async function markSelectedAsNew() {
    const ids = Array.from(selectedFileIds);
    if (!ids.length) {
        alert('Select files first');
        return;
    }
    await apiPost('/api/documents/batch/mark-new', { file_ids: ids });
    selectedFileIds.clear();
    loadDocuments();
}

async function markSelectedAsSkipped() {
    const ids = Array.from(selectedFileIds);
    if (!ids.length) {
        alert('Select files first');
        return;
    }
    await apiPost('/api/documents/batch/mark-skipped', { file_ids: ids });
    selectedFileIds.clear();
    loadDocuments();
}

async function checkActionButtonVisibility() {
    try {
        const status = await apiGet('/api/processing/status');
        const show = !status.is_running;
        document.getElementById('process-btn').style.display = show ? '' : 'none';
        document.getElementById('skip-btn').style.display = show ? '' : 'none';
        if (!show) {
            document.getElementById('multi-tag-btn').style.display = 'none';
        } else {
            updateMultiTagVisibility();
        }
    } catch {}
}

function updateMultiTagVisibility() {
    const btn = document.getElementById('multi-tag-btn');
    if (!btn) return;

    const ids = Array.from(selectedFileIds);
    if (ids.length < 2) {
        btn.style.display = 'none';
        return;
    }

    // All selected files must be on the current page
    const pageIds = new Set(lastLoadedFiles.map(f => f.id));
    const selectedOnPage = ids.filter(id => pageIds.has(id));
    if (selectedOnPage.length !== ids.length) {
        btn.style.display = 'none';
        return;
    }

    btn.style.display = '';
}

// Re-check visibility periodically (processing may start/stop)
setInterval(checkActionButtonVisibility, 3000);

// Expose globals for Library Panel cross-page communication
window.getSelectedFileIds = () => Array.from(selectedFileIds);
window.documentsRefresh = () => { loadStats(); loadDocuments(); };

async function openTagModal(fileId, filename) {
    const modal = document.getElementById('tag-modal');
    const content = document.getElementById('tag-modal-content');
    content.innerHTML = '<p>Loading tags...</p>';
    modal.showModal();

    // Get file's current tags
    const fileData = await apiGet(`/api/documents?search=${encodeURIComponent(filename)}&per_page=1`);
    const file = fileData.files?.[0];
    const currentTags = new Set((file?.tags || []).map(t => t.name));

    renderTagModalContent(fileId, filename, file, currentTags);
}

function renderTagModalContent(fileId, filename, file, currentTags, movedTag) {
    const content = document.getElementById('tag-modal-content');

    const assignedTags = allTagsList.filter(t => currentTags.has(t.name));
    const availableTags = allTagsList.filter(t => !currentTags.has(t.name));

    content.innerHTML = `
        <div class="tag-modal-open-file">
            <button class="open-file-btn outline" onclick="apiPost('/api/documents/${fileId}/open', {})">
                Open File
            </button>
            <button class="open-file-btn outline" onclick="apiPost('/api/documents/${fileId}/open-folder', {})">
                Open Folder
            </button>
            <span class="open-file-path" title="${escapeHtml(file?.relative_path || '')}">${escapeHtml(filename)}</span>
        </div>
        <div class="tag-modal-section-label">Assigned</div>
        <div class="tag-modal-pills" id="assigned-pills"></div>
        <hr class="tag-modal-divider">
        <div class="tag-modal-section-label">Available</div>
        <div class="tag-modal-pills" id="available-pills"></div>`;

    const assignedContainer = content.querySelector('#assigned-pills');
    const availableContainer = content.querySelector('#available-pills');

    if (!assignedTags.length) {
        assignedContainer.innerHTML = '<span class="tag-modal-empty">(none)</span>';
    } else {
        assignedTags.forEach(tag => {
            const pill = document.createElement('button');
            pill.className = 'tag-pill assigned' + (tag.name === movedTag ? ' pill-pop-in' : '');
            pill.title = tag.description || '';
            pill.style.backgroundColor = tag.color;
            pill.style.borderColor = tag.color;
            pill.textContent = `\u2715 ${tag.display_name}`;
            pill.addEventListener('click', () => {
                currentTags.delete(tag.name);
                renderTagModalContent(fileId, filename, file, currentTags, tag.name);
                apiDelete(`/api/documents/${fileId}/tags/${tag.name}`).then(async () => { loadDocuments(); loadStats(); allTagsList = await apiGet('/api/tags'); refreshTagFilter(); });
            });
            assignedContainer.appendChild(pill);
        });
    }

    if (!availableTags.length) {
        availableContainer.innerHTML = '<span class="tag-modal-empty">(none)</span>';
    } else {
        availableTags.forEach(tag => {
            const pill = document.createElement('button');
            pill.className = 'tag-pill available' + (tag.name === movedTag ? ' pill-pop-in' : '');
            pill.title = tag.description || '';
            pill.style.borderColor = tag.color;
            pill.style.color = tag.color;
            pill.textContent = `+ ${tag.display_name}`;
            pill.addEventListener('click', () => {
                currentTags.add(tag.name);
                renderTagModalContent(fileId, filename, file, currentTags, tag.name);
                apiPost(`/api/documents/${fileId}/tags/${tag.name}`, {}).then(async () => { loadDocuments(); loadStats(); allTagsList = await apiGet('/api/tags'); refreshTagFilter(); });
            });
            availableContainer.appendChild(pill);
        });
    }
}

function openMultiTagModal() {
    const ids = Array.from(selectedFileIds);
    const selectedFiles = lastLoadedFiles.filter(f => selectedFileIds.has(f.id));
    if (selectedFiles.length < 2) return;

    // Intersection: only tags present on ALL selected files
    const tagSets = selectedFiles.map(f => new Set((f.tags || []).map(t => t.name)));
    const currentTags = new Set(tagSets[0]);
    tagSets.slice(1).forEach(s => {
        for (const t of currentTags) if (!s.has(t)) currentTags.delete(t);
    });
    const fileIds = selectedFiles.map(f => f.id);

    const modal = document.getElementById('multi-tag-modal');
    renderMultiTagModalContent(fileIds, selectedFiles, currentTags);
    modal.showModal();
}

function renderMultiTagModalContent(fileIds, selectedFiles, currentTags, movedTag) {
    const content = document.getElementById('multi-tag-modal-content');
    const fileNames = selectedFiles.map(f => f.filename).join(', ');

    const assignedTags = allTagsList.filter(t => currentTags.has(t.name));
    const availableTags = allTagsList.filter(t => !currentTags.has(t.name));

    content.innerHTML = `
        <div class="multi-tag-file-list">
            <span class="multi-tag-count">${selectedFiles.length} files selected:</span>
            <span class="multi-tag-names">${escapeHtml(fileNames)}</span>
        </div>
        <div class="tag-modal-section-label">Assigned</div>
        <div class="tag-modal-pills" id="assigned-pills"></div>
        <hr class="tag-modal-divider">
        <div class="tag-modal-section-label">Available</div>
        <div class="tag-modal-pills" id="available-pills"></div>`;

    const assignedContainer = content.querySelector('#assigned-pills');
    const availableContainer = content.querySelector('#available-pills');

    if (!assignedTags.length) {
        assignedContainer.innerHTML = '<span class="tag-modal-empty">(none)</span>';
    } else {
        assignedTags.forEach(tag => {
            const pill = document.createElement('button');
            pill.className = 'tag-pill assigned' + (tag.name === movedTag ? ' pill-pop-in' : '');
            pill.title = tag.description || '';
            pill.style.backgroundColor = tag.color;
            pill.style.borderColor = tag.color;
            pill.textContent = `\u2715 ${tag.display_name}`;
            pill.addEventListener('click', () => {
                currentTags.delete(tag.name);
                renderMultiTagModalContent(fileIds, selectedFiles, currentTags, tag.name);
                apiDelete(`/api/documents/batch/tags/${tag.name}`, {file_ids: fileIds}).then(async () => { loadDocuments(); loadStats(); allTagsList = await apiGet('/api/tags'); refreshTagFilter(); });
            });
            assignedContainer.appendChild(pill);
        });
    }

    if (!availableTags.length) {
        availableContainer.innerHTML = '<span class="tag-modal-empty">(none)</span>';
    } else {
        availableTags.forEach(tag => {
            const pill = document.createElement('button');
            pill.className = 'tag-pill available' + (tag.name === movedTag ? ' pill-pop-in' : '');
            pill.title = tag.description || '';
            pill.style.borderColor = tag.color;
            pill.style.color = tag.color;
            pill.textContent = `+ ${tag.display_name}`;
            pill.addEventListener('click', () => {
                currentTags.add(tag.name);
                renderMultiTagModalContent(fileIds, selectedFiles, currentTags, tag.name);
                apiPost(`/api/documents/batch/tags/${tag.name}`, {file_ids: fileIds}).then(async () => { loadDocuments(); loadStats(); allTagsList = await apiGet('/api/tags'); refreshTagFilter(); });
            });
            availableContainer.appendChild(pill);
        });
    }
}

// ===== Tag Admin (Create & Edit) =====

async function openTagAdminModal() {
    const modal = document.getElementById('tag-admin-modal');
    const content = document.getElementById('tag-admin-content');
    content.innerHTML = '<p>Loading...</p>';
    modal.showModal();

    allTagsList = await apiGet('/api/tags');
    renderTagAdmin();
}

function renderTagAdmin() {
    const content = document.getElementById('tag-admin-content');
    const isProcessing = window.libraryIsProcessing || false;

    // Tags table
    let rows = '';
    allTagsList.forEach(tag => {
        rows += `
        <tr id="tag-admin-row-${tag.id}">
            <td class="col-color"><span class="tag-admin-swatch" style="background:${escapeHtml(tag.color)}"></span></td>
            <td class="col-name">${escapeHtml(tag.display_name)}</td>
            <td class="col-slug">${escapeHtml(tag.name)}</td>
            <td class="col-desc" title="${escapeHtml(tag.description || '')}">${escapeHtml(tag.description || '')}</td>
            <td class="col-files">${tag.file_count}</td>
            <td class="col-actions"><button class="outline tag-admin-edit-btn" onclick="toggleTagEdit(${tag.id})">Edit</button></td>
        </tr>
        <tr class="tag-admin-edit-row" id="tag-edit-form-${tag.id}" style="display:none">
            <td class="col-color">
                <input type="color" id="tag-edit-color-${tag.id}" value="${escapeHtml(tag.color)}">
            </td>
            <td class="col-name">
                <input type="text" id="tag-edit-display-${tag.id}" value="${escapeHtml(tag.display_name)}">
            </td>
            <td class="col-slug">
                <span>${escapeHtml(tag.name)}</span>
                <span class="tag-admin-note">immutable</span>
            </td>
            <td class="col-desc">
                <input type="text" id="tag-edit-desc-${tag.id}" value="${escapeHtml(tag.description || '')}">
            </td>
            <td class="col-files"></td>
            <td class="col-actions">
                <div class="tag-admin-edit-actions">
                    <button onclick="saveTagEdit(${tag.id})">Save</button>
                    <button class="outline" onclick="toggleTagEdit(${tag.id})">Cancel</button>
                    <button class="tag-admin-delete-btn" onclick="deleteTag('${tag.name}', '${escapeHtml(tag.display_name)}')">Delete</button>
                </div>
            </td>
        </tr>`;
    });

    // Create New Tag section
    const disabled = isProcessing ? 'disabled' : '';
    const processingNote = isProcessing ? ' <small>— Tag creation is disabled while processing is running.</small>' : '';

    let html = `
    <div class="tag-admin-table-wrap">
        <table class="tag-admin-table">
            <thead>
                <tr>
                    <th class="col-color">Color</th>
                    <th class="col-name">Display Name</th>
                    <th class="col-slug">Slug</th>
                    <th class="col-desc">Description</th>
                    <th class="col-files">Files</th>
                    <th class="col-actions"></th>
                </tr>
            </thead>
            <tbody>${rows}</tbody>
            <tfoot>
                <tr class="tag-admin-create-label-row">
                    <td colspan="6">CREATE NEW TAG${processingNote}</td>
                </tr>
                <tr class="tag-admin-create-row">
                    <td class="col-color"><input type="color" id="tag-create-color" value="#6c757d" ${disabled}></td>
                    <td class="col-name"><input type="text" id="tag-create-display" placeholder="Display Name" ${disabled}></td>
                    <td class="col-slug"><input type="text" id="tag-create-name" placeholder="slug_name" ${disabled}></td>
                    <td class="col-desc"><input type="text" id="tag-create-desc" placeholder="Description" ${disabled}></td>
                    <td class="col-files"></td>
                    <td class="col-actions"><button onclick="createNewTag()" ${disabled}>Create</button></td>
                </tr>
            </tfoot>
        </table>
    </div>`;

    content.innerHTML = html;
}

function toggleTagEdit(tagId) {
    const form = document.getElementById(`tag-edit-form-${tagId}`);
    if (form) {
        form.style.display = form.style.display === 'none' ? '' : 'none';
    }
}

async function saveTagEdit(tagId) {
    const displayName = document.getElementById(`tag-edit-display-${tagId}`).value.trim();
    const description = document.getElementById(`tag-edit-desc-${tagId}`).value.trim();
    const color = document.getElementById(`tag-edit-color-${tagId}`).value;

    if (!displayName) {
        alert('Display name is required.');
        return;
    }

    const result = await apiPut(`/api/tags/${tagId}`, {
        display_name: displayName,
        description: description || null,
        color: color,
    });

    if (result.error) {
        alert(result.error);
        return;
    }

    // Refresh tags and re-render
    allTagsList = await apiGet('/api/tags');
    renderTagAdmin();
    refreshTagFilter();
    loadDocuments();

    // Update chat page tag pills (same tab + other tabs)
    window.refreshChatTags?.();
    window.syncChannel.postMessage({ type: 'tags-changed', payload: {} });
}

async function deleteTag(tagName, displayName) {
    if (!confirm(`Delete "${displayName}"? This will remove it from all files.`)) return;

    const result = await apiDelete(`/api/tags/${tagName}`);
    if (result.error) {
        alert(result.error);
        return;
    }

    allTagsList = await apiGet('/api/tags');
    renderTagAdmin();
    refreshTagFilter();
    loadDocuments();

    window.refreshChatTags?.();
    window.syncChannel.postMessage({ type: 'tags-changed', payload: {} });
}

async function createNewTag() {
    const name = document.getElementById('tag-create-name').value.trim();
    const displayName = document.getElementById('tag-create-display').value.trim();
    const description = document.getElementById('tag-create-desc').value.trim();
    const color = document.getElementById('tag-create-color').value;

    if (!name) {
        alert('Internal name is required.');
        return;
    }
    if (!/^[a-z0-9_]+$/.test(name)) {
        alert('Internal name must contain only lowercase letters, numbers, and underscores.');
        return;
    }
    if (!displayName) {
        alert('Display name is required.');
        return;
    }

    const result = await apiPost('/api/tags', {
        name: name,
        display_name: displayName,
        description: description || null,
        color: color,
    });

    if (result.error) {
        alert(result.error);
        return;
    }

    // Refresh tags and re-render
    allTagsList = await apiGet('/api/tags');
    renderTagAdmin();
    refreshTagFilter();

    // Update chat page tag pills (same tab + other tabs)
    window.refreshChatTags?.();
    window.syncChannel.postMessage({ type: 'tags-changed', payload: {} });
}

function refreshTagFilter() {
    const select = document.getElementById('tag-filter');
    const currentValue = select.value;

    // Remove all options except "All tags"
    while (select.options.length > 1) {
        select.remove(1);
    }

    const noTagOpt = document.createElement('option');
    noTagOpt.value = '__none__';
    noTagOpt.textContent = 'No tag';
    select.appendChild(noTagOpt);

    allTagsList.forEach(tag => {
        const opt = document.createElement('option');
        opt.value = tag.name;
        opt.textContent = `${tag.display_name} (${tag.file_count})`;
        select.appendChild(opt);
    });

    // Restore previous selection if still valid
    select.value = currentValue;
}
