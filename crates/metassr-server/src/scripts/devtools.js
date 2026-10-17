/**
 * MetaSSR DevTools panel (development mode only).
 *
 * Connects to the dev WebSocket, collects debug events (API route traces,
 * API hot-reloads, build errors) and renders them in a floating panel.
 * Everything lives in a shadow root so application styles cannot leak in.
 */

(function () {
    if (window.__metassr_devtools_loaded__) return;
    window.__metassr_devtools_loaded__ = true;

    const MAX_EVENTS = 200;

    /// state ///

    let events = [];
    let nextLocalId = 1;
    let selectedId = null;
    let open = false;
    let ws = null;
    let reconnectTimer = null;

    /// dom ///

    const host = document.createElement('div');
    host.id = '__metassr_devtools__';
    host.style.cssText = 'all: initial; position: fixed; z-index: 2147483647;';
    document.body.appendChild(host);

    const shadow = host.attachShadow({ mode: 'open' });

    const style = document.createElement('style');
    style.textContent = `
        :host {
            all: initial;

            /* MetaSSR design tokens (site/src/styles/global.css) */
            --font-sans: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
            --font-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;

            --color-canvas: #f4f1ea;
            --color-surface: #fbf9f4;
            --color-ink: #1f2933;
            --color-ink-muted: #52616b;
            --color-accent: #006d77;
            --color-accent-strong: #00565e;
            --color-accent-contrast: #ffffff;
            --color-border: rgba(31, 41, 51, 0.12);
            --radius-pill: 999px;
            --radius-md: 8px;
            --shadow-sm: 0 4px 14px rgba(31, 41, 51, 0.16);
        }
        * {
            box-sizing: border-box;
            font-family: var(--font-sans);
        }
        .badge {
            position: fixed;
            right: 16px;
            bottom: 16px;
            display: flex;
            align-items: center;
            gap: 8px;
            padding: 8px 14px;
            background: var(--color-accent);
            color: var(--color-accent-contrast);
            border: none;
            border-radius: var(--radius-pill);
            font-family: var(--font-sans);
            font-size: 12px;
            font-weight: 700;
            letter-spacing: 0.02em;
            cursor: pointer;
            box-shadow: var(--shadow-sm);
            user-select: none;
            transition: background-color 0.2s, transform 0.2s;
        }
        .badge:hover { background: var(--color-accent-strong); }
        .badge:active { transform: scale(0.98); }
        .dot { width: 8px; height: 8px; border-radius: 50%; background: #a7e8df; }
        .dot.error { background: #ffb4a9; }
        .badge-count {
            padding: 1px 7px;
            border-radius: var(--radius-pill);
            background: rgba(255, 255, 255, 0.18);
            font-family: var(--font-mono);
            font-size: 11px;
            font-weight: 700;
        }
        .panel {
            position: fixed;
            top: 0;
            right: 0;
            bottom: 0;
            width: 460px;
            max-width: 100vw;
            display: flex;
            flex-direction: column;
            background: var(--color-canvas);
            color: var(--color-ink);
            border-left: 1px solid var(--color-border);
            box-shadow: -12px 0 32px rgba(31, 41, 51, 0.16);
            font-family: var(--font-sans);
            font-size: 12px;
            line-height: 1.5;
        }
        .panel[hidden] { display: none; }
        .header {
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 12px;
            padding: 14px 16px;
            border-bottom: 1px solid var(--color-border);
        }
        .title {
            display: flex;
            align-items: baseline;
            gap: 8px;
            font-weight: 700;
            font-size: 13px;
            letter-spacing: 0.02em;
        }
        .title .mark { color: var(--color-accent); }
        .actions { display: flex; gap: 8px; }
        .actions button {
            background: transparent;
            color: var(--color-ink-muted);
            border: 1px solid var(--color-border);
            border-radius: var(--radius-pill);
            padding: 5px 12px;
            font-family: var(--font-sans);
            font-size: 11px;
            font-weight: 700;
            cursor: pointer;
            transition: background-color 0.2s, color 0.2s;
        }
        .actions button:hover {
            background: rgba(31, 41, 51, 0.06);
            color: var(--color-ink);
        }
        .tabs {
            display: flex;
            gap: 6px;
            padding: 10px 16px;
            border-bottom: 1px solid var(--color-border);
        }
        .tab {
            background: transparent;
            border: none;
            color: var(--color-ink-muted);
            padding: 4px 12px;
            border-radius: var(--radius-pill);
            font-family: var(--font-sans);
            font-size: 11px;
            font-weight: 700;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            cursor: pointer;
        }
        .tab.active {
            background: rgba(0, 109, 119, 0.12);
            color: var(--color-accent);
        }
        .content { position: relative; flex: 1; overflow: hidden; display: flex; }
        .events { flex: 1; overflow: auto; }
        .event {
            display: flex;
            align-items: center;
            gap: 8px;
            padding: 9px 16px;
            border-bottom: 1px solid rgba(31, 41, 51, 0.06);
            cursor: pointer;
            white-space: nowrap;
            font-family: var(--font-mono);
        }
        .event:nth-child(even) { background: rgba(31, 41, 51, 0.035); }
        .event:hover { background: rgba(31, 41, 51, 0.06); }
        .event.selected { background: rgba(0, 109, 119, 0.12); }
        .method {
            flex: none;
            font-weight: 700;
            font-size: 10px;
            letter-spacing: 0.04em;
            padding: 2px 8px;
            border-radius: var(--radius-pill);
            background: rgba(0, 109, 119, 0.12);
            color: var(--color-accent);
        }
        .route {
            flex: 1;
            overflow: hidden;
            text-overflow: ellipsis;
            color: var(--color-ink);
        }
        .status { font-size: 11px; font-weight: 700; }
        .status.ok { color: #2f855a; }
        .status.redirect { color: #b7791f; }
        .status.client-error { color: #c05621; }
        .status.server-error { color: #c53030; }
        .duration { color: var(--color-ink-muted); font-size: 11px; }
        .loader {
            flex: none;
            color: var(--color-ink-muted);
            font-size: 10px;
            font-weight: 700;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }
        .system {
            padding: 9px 16px;
            color: var(--color-ink-muted);
            border-bottom: 1px solid rgba(31, 41, 51, 0.06);
            white-space: pre-wrap;
            word-break: break-word;
            font-family: var(--font-mono);
            font-size: 11px;
        }
        .system.error { color: #c53030; }
        .empty {
            padding: 32px 24px;
            color: var(--color-ink-muted);
            text-align: center;
            font-family: var(--font-sans);
        }
        .detail {
            position: absolute;
            inset: 0;
            z-index: 1;
            overflow: auto;
            padding: 14px 16px;
            background: var(--color-surface);
        }
        .detail .back {
            margin-bottom: 12px;
            background: transparent;
            color: var(--color-accent);
            border: 1px solid var(--color-border);
            border-radius: var(--radius-pill);
            padding: 5px 12px;
            font-family: var(--font-sans);
            font-size: 11px;
            font-weight: 700;
            cursor: pointer;
            transition: background-color 0.2s;
        }
        .detail .back:hover { background: rgba(0, 109, 119, 0.12); }
        .detail h3 {
            margin: 0 0 8px;
            color: var(--color-accent);
            font-family: var(--font-sans);
            font-size: 11px;
            font-weight: 700;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }
        .detail .field { margin-bottom: 12px; }
        .detail .field .label {
            color: var(--color-ink-muted);
            font-family: var(--font-sans);
            font-size: 10px;
            font-weight: 700;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }
        .detail .field .value {
            color: var(--color-ink);
            font-family: var(--font-mono);
            word-break: break-all;
        }
        .detail pre {
            margin: 4px 0 14px;
            padding: 10px;
            background: rgba(31, 41, 51, 0.05);
            border: 1px solid var(--color-border);
            border-radius: var(--radius-md);
            overflow-x: auto;
            white-space: pre-wrap;
            word-break: break-word;
            color: var(--color-ink);
            font-family: var(--font-mono);
            font-size: 11px;
        }
        .footer {
            padding: 8px 16px;
            border-top: 1px solid var(--color-border);
            color: var(--color-ink-muted);
            font-size: 11px;
        }
    `;
    shadow.appendChild(style);

    const badge = document.createElement('div');
    badge.className = 'badge';
    badge.id = 'metassr-badge';
    badge.title = 'MetaSSR DevTools';
    const dot = document.createElement('span');
    dot.className = 'dot';
    const badgeLabel = document.createElement('span');
    badgeLabel.textContent = 'MetaSSR';
    const badgeCount = document.createElement('span');
    badgeCount.className = 'badge-count';
    badgeCount.id = 'metassr-count';
    badgeCount.textContent = '0';
    badge.append(dot, badgeLabel, badgeCount);
    shadow.appendChild(badge);

    const panel = document.createElement('aside');
    panel.className = 'panel';
    panel.id = 'metassr-panel';
    panel.hidden = true;

    const header = document.createElement('header');
    header.className = 'header';
    const title = document.createElement('div');
    title.className = 'title';
    const titleMark = document.createElement('span');
    titleMark.className = 'mark';
    titleMark.textContent = '◆';
    const titleText = document.createElement('span');
    titleText.textContent = 'MetaSSR DevTools';
    title.append(titleMark, titleText);
    const actions = document.createElement('div');
    actions.className = 'actions';
    const clearButton = document.createElement('button');
    clearButton.id = 'metassr-clear';
    clearButton.textContent = 'Clear';
    const closeButton = document.createElement('button');
    closeButton.id = 'metassr-close';
    closeButton.textContent = 'Close';
    actions.append(clearButton, closeButton);
    header.append(title, actions);

    const tabs = document.createElement('nav');
    tabs.className = 'tabs';
    const apiTab = document.createElement('button');
    apiTab.className = 'tab active';
    apiTab.dataset.tab = 'api';
    apiTab.textContent = 'API';
    tabs.appendChild(apiTab);

    const content = document.createElement('div');
    content.className = 'content';
    const eventsList = document.createElement('div');
    eventsList.className = 'events';
    eventsList.id = 'metassr-events';
    const detail = document.createElement('div');
    detail.className = 'detail';
    detail.id = 'metassr-detail';
    detail.hidden = true;
    content.append(eventsList, detail);

    const footer = document.createElement('footer');
    footer.className = 'footer';
    footer.id = 'metassr-status';
    footer.textContent = 'connecting…';

    panel.append(header, tabs, content, footer);
    shadow.appendChild(panel);

    /// rendering ///

    function stripAnsi(text) {
        return (text || '').replace(
            /(\\u(?:\{1b}|001b)|[\u001b\u009b])[[\]()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]/g,
            ''
        );
    }

    function statusClass(status) {
        if (status >= 500) return 'server-error';
        if (status >= 400) return 'client-error';
        if (status >= 300) return 'redirect';
        return 'ok';
    }

    function hasError() {
        return events.some((event) => event.kind === 'api_trace' && event.error);
    }

    function render() {
        badgeCount.textContent = String(events.length);
        dot.classList.toggle('error', hasError());

        eventsList.textContent = '';

        if (events.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'No events yet. Requests to /api/* will show up here.';
            eventsList.appendChild(empty);
        }

        for (const event of events) {
            if (event.render === 'system') {
                const row = document.createElement('div');
                row.className = 'system' + (event.error ? ' error' : '');
                row.textContent = event.message;
                eventsList.appendChild(row);
                continue;
            }

            const row = document.createElement('div');
            row.className = 'event' + (event.id === selectedId ? ' selected' : '');
            row.dataset.id = String(event.id);
            row.dataset.route = event.route;

            const method = document.createElement('span');
            method.className = 'method ' + (event.method || '').toLowerCase();
            method.textContent = event.method;

            const route = document.createElement('span');
            route.className = 'route';
            route.textContent = event.route;

            const status = document.createElement('span');
            if (event.status != null) {
                status.className = 'status ' + statusClass(event.status);
                status.textContent = String(event.status);
            } else {
                status.className = 'status server-error';
                status.textContent = 'ERR';
            }

            const duration = document.createElement('span');
            duration.className = 'duration';
            duration.textContent = `${(event.duration_ms || 0).toFixed(1)}ms`;

            const loader = document.createElement('span');
            loader.className = 'loader';
            loader.textContent = event.loader;

            row.append(method, route, status, duration, loader);
            row.addEventListener('click', () => select(event.id));
            eventsList.appendChild(row);
        }

        renderDetail();
    }

    function renderDetail() {
        detail.textContent = '';
        const event = events.find((candidate) => candidate.id === selectedId);

        if (!event || event.render === 'system') {
            detail.hidden = true;
            return;
        }

        detail.hidden = false;

        const backButton = document.createElement('button');
        backButton.className = 'back';
        backButton.id = 'metassr-back';
        backButton.textContent = '← All events';
        backButton.addEventListener('click', () => {
            selectedId = null;
            render();
        });
        detail.appendChild(backButton);

        const addField = (label, value) => {
            if (value == null) return;
            const field = document.createElement('div');
            field.className = 'field';
            const labelEl = document.createElement('div');
            labelEl.className = 'label';
            labelEl.textContent = label;
            const valueEl = document.createElement('div');
            valueEl.className = 'value';
            valueEl.textContent = String(value);
            field.append(labelEl, valueEl);
            detail.appendChild(field);
        };

        addField('Route', `${event.method} ${event.route}`);
        addField('Loader', event.loader);
        addField('File', event.file);
        if (event.status != null) addField('Status', event.status);
        addField('Duration', `${(event.duration_ms || 0).toFixed(2)}ms`);

        const addJson = (label, raw) => {
            if (raw == null) return;
            const heading = document.createElement('h3');
            heading.textContent = label;
            const pre = document.createElement('pre');
            try {
                pre.textContent = JSON.stringify(JSON.parse(raw), null, 2);
            } catch {
                pre.textContent = raw;
            }
            detail.append(heading, pre);
        };

        addJson('Request', event.request);
        addJson('Response', event.response);

        if (event.error) {
            const heading = document.createElement('h3');
            heading.textContent = 'Error';
            const pre = document.createElement('pre');
            pre.textContent = stripAnsi(event.error);
            detail.append(heading, pre);
        }
    }

    function select(id) {
        selectedId = selectedId === id ? null : id;
        render();
    }

    function addEvent(event) {
        events.unshift(event);
        if (events.length > MAX_EVENTS) events.length = MAX_EVENTS;
        render();
    }

    function addSystem(message, error) {
        addEvent({ id: `system-${nextLocalId++}`, render: 'system', message, error });
    }

    /// websocket ///

    function connect() {
        if (ws) ws.close();

        ws = new WebSocket('ws://localhost:__WS_PORT__');

        ws.onopen = () => {
            footer.textContent = 'connected';
            ws.send(JSON.stringify({ type: 'ready' }));
        };

        ws.onmessage = (event) => {
            let message;
            try {
                message = JSON.parse(event.data);
            } catch {
                return;
            }

            switch (message.type) {
                case 'debug':
                    if (message.event && message.event.kind === 'api_trace') {
                        addEvent(message.event);
                    }
                    break;
                case 'debug_snapshot':
                    events = (message.events || []).filter(
                        (snapshotEvent) => snapshotEvent.kind === 'api_trace'
                    );
                    render();
                    break;
                case 'api':
                    addSystem(`↻ API route reloaded: ${message.path}`);
                    break;
                case 'build_error':
                    addSystem(`✗ Build error: ${(message.errors || []).map(stripAnsi).join('\n')}`, true);
                    break;
            }
        };

        ws.onclose = () => {
            footer.textContent = 'disconnected, reconnecting…';
            if (reconnectTimer) return;
            reconnectTimer = setTimeout(() => {
                reconnectTimer = null;
                connect();
            }, 1000);
        };

        ws.onerror = () => {
            footer.textContent = 'connection error';
        };
    }

    /// interaction ///

    function togglePanel(next) {
        open = next != null ? next : !open;
        panel.hidden = !open;
    }

    badge.addEventListener('click', () => togglePanel());
    closeButton.addEventListener('click', () => togglePanel(false));
    clearButton.addEventListener('click', () => {
        events = [];
        selectedId = null;
        render();
    });

    document.addEventListener('keydown', (event) => {
        if (event.key === 'Escape') togglePanel(false);
    });

    render();
    connect();
})();
