// Global system status: polls /api/system/status, drives the nav pill,
// the Schedule dialog, the End Downtime button, and a body-level
// data-downtime attribute that page-level CSS uses to grey out chat/wiki UIs.

(function () {
    const POLL_MS = 3000;
    let pollTimer = null;
    let lastStatus = null;

    function fmtHHMM(iso) {
        if (!iso) return '';
        try {
            const d = new Date(iso);
            return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
        } catch { return ''; }
    }

    function applyStatus(s) {
        lastStatus = s;
        const pill = document.getElementById('sys-status-pill');
        const endBtn = document.getElementById('sys-end-downtime-btn');
        if (!pill || !endBtn) return;

        // Reset classes
        pill.classList.remove(
            'sys-pill-ok', 'sys-pill-busy', 'sys-pill-warn',
            'sys-pill-down', 'sys-pill-paused', 'sys-pill-clickable',
        );

        // Severity → CSS class
        const sevClass = {
            ok: 'sys-pill-ok',
            busy: 'sys-pill-busy',
            warn: 'sys-pill-warn',
            down: 'sys-pill-down',
        }[s.state_severity] || 'sys-pill-ok';
        pill.classList.add(sevClass);

        // "LLMs paused" is severity=ok but visually distinct (dim/dotted) — and
        // clickable to restart. "LLMs idle" is also clickable to pause.
        const isPaused = s.state_label === 'LLMs paused';
        const isIdle = s.state_label === 'LLMs idle';
        if (isPaused) pill.classList.add('sys-pill-paused');
        if (isPaused || isIdle) {
            pill.classList.add('sys-pill-clickable');
            pill.title = isPaused
                ? 'Click to start local models'
                : 'Click to pause local models (free ~7-10 GB of RAM)';
        } else {
            pill.title = '';
        }

        // Label: backend computed; for downtime, append "— resumes HH:MM"
        let label = s.state_label || 'LLMs idle';
        if (s.in_downtime) {
            const t = fmtHHMM(s.next_boundary);
            if (t) label = `${label} — resumes ${t}`;
        }
        pill.textContent = label;

        if (s.in_downtime) {
            endBtn.style.display = '';
            document.body.setAttribute('data-downtime', 'true');
        } else {
            endBtn.style.display = 'none';
            document.body.removeAttribute('data-downtime');
        }

        // Disable chat send when in downtime
        const chatSendBtn = document.getElementById('send-btn');
        const chatInput = document.getElementById('chat-input');
        if (chatSendBtn && chatInput) {
            if (s.in_downtime) {
                chatSendBtn.disabled = true;
                chatInput.disabled = true;
                if (!chatInput.dataset.prevPlaceholder) {
                    chatInput.dataset.prevPlaceholder = chatInput.placeholder || '';
                }
                const t = fmtHHMM(s.next_boundary);
                chatInput.placeholder = t
                    ? `Servers paused until ${t} — End Downtime to use chat now.`
                    : 'Servers paused — End Downtime to use chat now.';
            } else if (chatInput.dataset.prevPlaceholder !== undefined) {
                chatInput.disabled = false;
                chatSendBtn.disabled = false;
                chatInput.placeholder = chatInput.dataset.prevPlaceholder;
                delete chatInput.dataset.prevPlaceholder;
            }
        }
    }

    async function pollOnce() {
        try {
            const s = await apiGet('/api/system/status');
            applyStatus(s);
        } catch (e) {
            // Backend unreachable — leave pill as-is
        }
    }

    function startPolling() {
        if (pollTimer) clearInterval(pollTimer);
        pollOnce();
        pollTimer = setInterval(pollOnce, POLL_MS);
    }

    // ── Schedule dialog ─────────────────────────────────────────────────

    async function openScheduleDialog() {
        const dlg = document.getElementById('schedule-dialog');
        if (!dlg) return;
        const settings = await apiGet('/api/system/settings');
        document.getElementById('sched-enabled-input').checked = !!settings.schedule_enabled;
        document.getElementById('sched-start-input').value = settings.downtime_start || '06:30';
        document.getElementById('sched-end-input').value = settings.downtime_end || '09:30';
        document.getElementById('sched-autoshutdown-input').checked = !!settings.auto_shutdown_on_manual_pause;
        dlg.showModal();
    }

    async function saveScheduleSettings() {
        const payload = {
            schedule_enabled: document.getElementById('sched-enabled-input').checked,
            downtime_start: document.getElementById('sched-start-input').value || '06:30',
            downtime_end: document.getElementById('sched-end-input').value || '09:30',
            auto_shutdown_on_manual_pause: document.getElementById('sched-autoshutdown-input').checked,
        };
        const res = await apiPut('/api/system/settings', payload);
        if (res && res.ok) {
            document.getElementById('schedule-dialog').close();
            pollOnce();
        } else {
            alert(res && res.error ? res.error : 'Failed to save settings');
        }
    }

    async function endDowntimeNow() {
        const btn = document.getElementById('sys-end-downtime-btn');
        if (btn) { btn.disabled = true; btn.setAttribute('aria-busy', 'true'); }
        try {
            await apiPost('/api/system/end-downtime');
            // Optimistic poll
            setTimeout(pollOnce, 1000);
        } finally {
            if (btn) {
                btn.disabled = false;
                btn.removeAttribute('aria-busy');
            }
        }
    }

    // ── Confirm dialog (Promise-returning) ──────────────────────────────

    function confirmDialog({ title, body, confirmLabel = 'Confirm', cancelLabel = 'Cancel', danger = false } = {}) {
        return new Promise((resolve) => {
            const dlg = document.getElementById('confirm-dialog');
            if (!dlg) { resolve(window.confirm(`${title}\n\n${body}`)); return; }
            dlg.querySelector('.confirm-dialog-title').textContent = title;
            dlg.querySelector('.confirm-dialog-body').textContent = body;
            const confirmBtn = dlg.querySelector('.confirm-dialog-confirm');
            const cancelBtn = dlg.querySelector('.confirm-dialog-cancel');
            confirmBtn.textContent = confirmLabel;
            cancelBtn.textContent = cancelLabel;
            confirmBtn.classList.toggle('danger', !!danger);
            const done = (value) => {
                dlg.close();
                confirmBtn.removeEventListener('click', onOk);
                cancelBtn.removeEventListener('click', onCancel);
                resolve(value);
            };
            const onOk = () => done(true);
            const onCancel = () => done(false);
            confirmBtn.addEventListener('click', onOk);
            cancelBtn.addEventListener('click', onCancel);
            dlg.showModal();
        });
    }

    async function onPillClick() {
        if (!lastStatus) return;
        if (lastStatus.state_label === 'LLMs idle') {
            const ok = await confirmDialog({
                title: 'Pause local models?',
                body: 'About 7-10 GB of memory will be freed. They will auto-resume the next time you use Chat, Documents, or Wiki — or click the pill to start them manually.',
                confirmLabel: 'Pause',
            });
            if (!ok) return;
            try {
                const res = await fetch('/api/system/llama/pause', { method: 'POST' });
                const data = await res.json();
                if (!res.ok) {
                    alert(data.detail || 'Could not pause local models.');
                }
                setTimeout(pollOnce, 500);
            } catch (e) {
                alert('Could not pause local models: ' + (e.message || e));
            }
        } else if (lastStatus.state_label === 'LLMs paused') {
            const ok = await confirmDialog({
                title: 'Start local models?',
                body: 'Local models take about a minute to load. The status pill will show progress.',
                confirmLabel: 'Start',
            });
            if (!ok) return;
            try {
                await apiPost('/api/system/llama/start');
                setTimeout(pollOnce, 500);
            } catch (e) {
                alert('Could not start local models: ' + (e.message || e));
            }
        }
    }

    // Public hooks
    window.systemStatus = {
        get: () => lastStatus,
        refresh: pollOnce,
        openScheduleDialog,
        confirm: confirmDialog,
    };

    // Wire up once DOM is ready
    function init() {
        document.getElementById('sys-schedule-btn')?.addEventListener('click', openScheduleDialog);
        document.getElementById('sys-end-downtime-btn')?.addEventListener('click', endDowntimeNow);
        document.getElementById('sys-status-pill')?.addEventListener('click', onPillClick);
        document.getElementById('sched-save-btn')?.addEventListener('click', saveScheduleSettings);
        document.querySelectorAll('.close-schedule-dialog').forEach(btn => {
            btn.addEventListener('click', () => document.getElementById('schedule-dialog').close());
        });
        startPolling();
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
