// Global system status: polls /api/system/status, drives the nav pill,
// the Schedule dialog, the End Downtime button, and a body-level
// data-downtime attribute that page-level CSS uses to grey out chat UIs.

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

    function fmtHHMMFromSetting(hhmm) {
        // Convert "HH:MM" (24h, local) into the user's locale-formatted time.
        if (!hhmm || !hhmm.includes(':')) return '';
        const [h, m] = hhmm.split(':').map(Number);
        if (Number.isNaN(h) || Number.isNaN(m)) return '';
        const d = new Date();
        d.setHours(h, m, 0, 0);
        return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
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
                : 'Click to pause local models and free their memory';
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

        // During downtime the End Downtime button replaces the Schedule button
        // (one control in that slot, not both).
        const scheduleBtn = document.getElementById('sys-schedule-btn');
        if (s.in_downtime) {
            endBtn.style.display = '';
            if (scheduleBtn) scheduleBtn.style.display = 'none';
            document.body.setAttribute('data-downtime', 'true');
        } else {
            endBtn.style.display = 'none';
            if (scheduleBtn) scheduleBtn.style.display = '';
            document.body.removeAttribute('data-downtime');
        }

        // Upcoming-event pill: only while running ("Pausing at …"). During
        // downtime the main pill already shows "resumes …", so repeating it here
        // would just duplicate that information.
        const eventPill = document.getElementById('sys-next-event-pill');
        if (eventPill) {
            if (s.schedule_enabled && !s.in_downtime) {
                eventPill.textContent = `Pausing at ${fmtHHMMFromSetting(s.downtime_start)}`;
                eventPill.style.display = '';
            } else {
                eventPill.style.display = 'none';
            }
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
        // Open immediately, then fill the fields once settings arrive.
        dlg.showModal();
        const restore = showLoader(dlg.querySelector('article'), 'Loading settings…');
        try {
            const settings = await apiGet('/api/system/settings');
            document.getElementById('sched-enabled-input').checked = !!settings.schedule_enabled;
            document.getElementById('sched-start-input').value = settings.downtime_start || '06:30';
            document.getElementById('sched-end-input').value = settings.downtime_end || '09:30';
        } finally {
            restore();
        }
    }

    async function saveScheduleSettings() {
        const payload = {
            schedule_enabled: document.getElementById('sched-enabled-input').checked,
            downtime_start: document.getElementById('sched-start-input').value || '06:30',
            downtime_end: document.getElementById('sched-end-input').value || '09:30',
        };
        const restore = setBtnBusy(document.getElementById('sched-save-btn'), 'Saving…');
        let res;
        try {
            res = await apiPut('/api/system/settings', payload);
        } catch (e) {
            restore();
            alert('Failed to save settings');
            return;
        }
        if (res && res.ok) {
            document.getElementById('schedule-dialog').close();
            restore();
            pollOnce();
        } else {
            restore();
            alert(res && res.error ? res.error : 'Failed to save settings');
        }
    }

    async function endDowntimeNow() {
        const restore = setBtnBusy(document.getElementById('sys-end-downtime-btn'), null);
        // Optimistic: reflect "leaving downtime / starting LLMs" immediately so
        // the UI doesn't sit on the paused state while llama restarts in the
        // background. The next status poll reconciles the real state.
        const prev = lastStatus;
        if (prev) {
            applyStatus({
                ...prev,
                in_downtime: false,
                llama_state: 'starting',
                state_label: 'Starting LLMs',
                state_severity: 'busy',
            });
        }
        try {
            await apiPost('/api/system/end-downtime');
        } catch (e) {
            if (prev) applyStatus(prev);  // roll back on failure
        } finally {
            restore();
            setTimeout(pollOnce, 1500);
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

    // What the pause frees depends on what is loaded, and the chat model's size
    // depends on the context window of the Retrieval Depth tier it was loaded
    // for. Both are read live from the backend rather than estimated.
    async function pauseDialogBody() {
        const resume = 'The models start again the next time you use Chat or Documents, or when you click the pill.';
        let mem = null, depths = null;
        try {
            [mem, depths] = await Promise.all([
                apiGet('/api/system/llama/memory'),
                apiGet('/api/system/chat-depths'),
            ]);
        } catch (e) { /* fall through to the generic text */ }
        if (!mem || !mem.total_mib) {
            return 'The local models will be unloaded to free their memory. ' + resume;
        }

        const gb = (mib) => (mib / 1024).toFixed(1) + ' GB';
        const tokens = (v) => Number(v).toLocaleString('en-US');
        const levels = (depths && depths.levels) || [];
        const shortName = (level) => level.label.split(' ')[0];

        const loaded = [];
        const fixedContext = mem.chat_backend !== 'mlx';  // MLX grows its cache on demand
        if (mem.chat_mib) {
            let line = (mem.chat_model ? mem.chat_model + ' ' : '') + 'chat model';
            if (fixedContext && mem.chat_ctx) {
                const tier = levels.filter(l => l.ctx === mem.chat_ctx).map(shortName);
                line += ` with a ${tokens(mem.chat_ctx)}-token context`;
                if (tier.length) line += `, the size for ${tier.join(' and ')},`;
            }
            loaded.push(`${line} using ${gb(mem.chat_mib)}`);
        }
        if (mem.embed_mib) loaded.push(`embedding model using ${gb(mem.embed_mib)}`);
        const lines = ['Loaded now: ' + loaded.join('; ') + '.'];

        let storedDepth = null;
        try { storedDepth = localStorage.getItem('georag-depth'); } catch (e) { /* storage blocked */ }
        const setLevel = levels.find(l => l.key === (storedDepth || (depths && depths.default)));
        if (setLevel) {
            let line = `Retrieval Depth is set to ${setLevel.label}.`;
            if (fixedContext && mem.chat_ctx && setLevel.ctx < mem.chat_ctx) {
                line += ` It only needs a ${tokens(setLevel.ctx)}-token context, so the chat model will take less memory when it starts again.`;
            }
            lines.push(line);
        }
        lines.push(`Pausing frees about ${gb(mem.total_mib)}. ${resume}`);
        return lines.join('\n\n');
    }

    async function onPillClick() {
        if (!lastStatus) return;
        if (lastStatus.state_label === 'LLMs idle') {
            const ok = await confirmDialog({
                title: 'Pause local models?',
                body: await pauseDialogBody(),
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
