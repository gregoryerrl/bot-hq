// bot-hq workspace — Phase N v3.x-1 curation frontend.
// Replaces v3b file-tree-as-nav with destination-allowlist nav per
// scope-lock-v4.2 (Form Y). Vanilla JS + fetch. marked.js (CDN) for
// .md render; raw textarea for .yaml + non-md content.
//
// State machine: project-picker drives /api/destinations?project=<p>;
// click on a destination's file loads /api/files/<path>?format=json into
// the doc pane. Edit + Save + 409 conflict resolution preserved from v3c.

(function () {
  'use strict';

  const projectPicker = document.getElementById('project-picker');
  const activeChip = document.getElementById('active-project-chip');
  const navSearch = document.getElementById('nav-search');
  const navGlobal = document.getElementById('nav-global-list');
  const navProject = document.getElementById('nav-project-list');
  const navRecent = document.getElementById('nav-recent-list');
  const docPath = document.getElementById('doc-path');
  const docMtime = document.getElementById('doc-mtime');
  const docContent = document.getElementById('doc-content');
  const docRendered = document.getElementById('doc-rendered');
  const docDirty = document.getElementById('doc-dirty');
  const docSave = document.getElementById('doc-save');
  const docMode = document.getElementById('doc-mode');
  const docStatus = document.getElementById('doc-status');
  const conflictModal = document.getElementById('conflict-modal');
  const conflictServer = document.getElementById('conflict-server-content');

  const state = {
    project: 'bot-hq',
    currentPath: null,
    currentMtime: null,
    pristine: '',
    pendingConflict: null,
    viewMode: 'rendered', // 'rendered' or 'raw'
    searchQuery: '', // project-scoped filename filter (lowercase)
  };

  projectPicker.addEventListener('change', () => {
    state.project = projectPicker.value;
    activeChip.textContent = state.project;
    loadDestinations();
  });
  navSearch.addEventListener('input', () => {
    state.searchQuery = navSearch.value.trim().toLowerCase();
    applyNavFilter();
  });
  docContent.addEventListener('input', onEdit);
  docSave.addEventListener('click', saveFile);
  docMode.addEventListener('click', toggleViewMode);
  document.getElementById('conflict-overwrite').addEventListener('click', resolveConflict.bind(null, 'overwrite'));
  document.getElementById('conflict-discard').addEventListener('click', resolveConflict.bind(null, 'discard'));
  document.getElementById('conflict-keep').addEventListener('click', resolveConflict.bind(null, 'keep'));
  document.getElementById('register-project-open').addEventListener('click', openRegisterModal);
  document.getElementById('register-project-submit').addEventListener('click', submitRegisterProject);
  document.getElementById('register-project-cancel').addEventListener('click', closeRegisterModal);
  document.getElementById('search-open').addEventListener('click', openSearchModal);
  document.getElementById('search-close').addEventListener('click', closeSearchModal);
  document.getElementById('search-input').addEventListener('input', onSearchInput);
  document.addEventListener('keydown', (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key === 'k') {
      ev.preventDefault();
      openSearchModal();
    } else if (ev.key === 'Escape') {
      closeSearchModal();
    }
  });

  loadProjects().then(loadDestinations);
  loadRecentEdits();

  async function loadProjects() {
    try {
      const res = await fetch('/api/projects');
      const data = await res.json();
      projectPicker.innerHTML = '';
      for (const p of data.projects || []) {
        const opt = document.createElement('option');
        opt.value = p.name;
        opt.textContent = p.name;
        projectPicker.appendChild(opt);
      }
      if (projectPicker.options.length) {
        projectPicker.value = state.project;
        activeChip.textContent = state.project;
      }
    } catch (err) {
      navGlobal.innerHTML = '<em class="error">Failed to load projects: ' + escapeHtml(err.message) + '</em>';
    }
  }

  async function loadDestinations() {
    navGlobal.innerHTML = '<em>Loading…</em>';
    navProject.innerHTML = '<em>Loading…</em>';
    try {
      const res = await fetch('/api/destinations?project=' + encodeURIComponent(state.project));
      const data = await res.json();
      const dests = data.destinations || [];
      navGlobal.innerHTML = '';
      navProject.innerHTML = '';
      for (const d of dests) {
        const target = d.section === 'global' ? navGlobal : navProject;
        target.appendChild(renderDestination(d));
      }
      applyNavFilter();
    } catch (err) {
      navGlobal.innerHTML = '<em class="error">Failed to load destinations: ' + escapeHtml(err.message) + '</em>';
    }
  }

  // applyNavFilter hides nav <li> entries whose file name doesn't match
  // state.searchQuery (substring, case-insensitive). Empty query restores
  // full visibility. Per-destination "(no matches)" hint shows when a
  // destination's items all filter out so the destination header still
  // surfaces context. Project-scoped per scope-lock-v4.2 affordance #2.
  function applyNavFilter() {
    const q = state.searchQuery;
    const dests = document.querySelectorAll('.dest');
    dests.forEach((dest) => {
      const items = dest.querySelectorAll('.dest-list li');
      let visibleCount = 0;
      items.forEach((li) => {
        if (li.classList.contains('empty') || li.classList.contains('no-match')) return;
        const a = li.querySelector('a');
        const name = a ? a.textContent.toLowerCase() : (li.textContent || '').toLowerCase();
        const match = !q || name.includes(q);
        li.classList.toggle('hidden', !match);
        if (match) visibleCount++;
      });
      // Manage the dynamic "(no matches)" indicator.
      let indicator = dest.querySelector('li.no-match');
      const baseEmpty = dest.querySelector('li.empty');
      if (q && visibleCount === 0 && !baseEmpty) {
        if (!indicator) {
          indicator = document.createElement('li');
          indicator.classList.add('no-match');
          indicator.textContent = '(no matches)';
          dest.querySelector('.dest-list').appendChild(indicator);
        }
        indicator.classList.remove('hidden');
      } else if (indicator) {
        indicator.classList.add('hidden');
      }
    });
  }

  function renderDestination(dest) {
    const wrap = document.createElement('div');
    wrap.classList.add('dest');
    const head = document.createElement('div');
    head.classList.add('dest-head');
    head.textContent = dest.name;
    wrap.appendChild(head);
    const ul = document.createElement('ul');
    ul.classList.add('dest-list');
    const nodes = dest.nodes || [];
    if (!nodes.length) {
      const li = document.createElement('li');
      li.classList.add('empty');
      li.textContent = '(empty)';
      ul.appendChild(li);
    } else {
      for (const n of nodes) {
        const li = document.createElement('li');
        if (n.missing) {
          li.classList.add('missing');
          li.textContent = n.name + ' (not yet authored)';
        } else {
          const a = document.createElement('a');
          a.href = '#' + n.path;
          a.classList.add('file-link');
          a.dataset.path = n.path;
          a.textContent = n.name;
          a.title = (n.size != null ? n.size + ' B · ' : '') + (n.mtime || '');
          a.addEventListener('click', (ev) => {
            ev.preventDefault();
            if (state.currentPath && isDirty()) {
              if (!confirm('You have unsaved changes. Discard and load new file?')) return;
            }
            loadFile(n.path);
          });
          li.appendChild(a);
        }
        ul.appendChild(li);
      }
    }
    wrap.appendChild(ul);
    return wrap;
  }

  async function loadFile(path) {
    docPath.textContent = path;
    docMtime.textContent = 'loading…';
    docContent.value = '';
    docRendered.innerHTML = '';
    docContent.disabled = true;
    docSave.disabled = true;
    docDirty.classList.add('hidden');
    docStatus.textContent = '';
    try {
      const res = await fetch('/api/files/' + path + '?format=json');
      if (!res.ok) {
        docContent.value = 'Error: ' + res.status + ' ' + res.statusText;
        docMtime.textContent = '';
        showRawView();
        return;
      }
      const data = await res.json();
      state.currentPath = path;
      state.currentMtime = data.mtime || '';
      state.pristine = data.content || '';
      docContent.value = state.pristine;
      docContent.disabled = false;
      docMtime.textContent = data.mtime || '';
      if (hasRenderedMode(path) && state.viewMode === 'rendered') {
        showRenderedView();
      } else if (hasRenderedMode(path) && state.viewMode === 'split') {
        showSplitView();
      } else {
        // Non-renderable always raw; renderable follows current viewMode.
        if (!hasRenderedMode(path)) state.viewMode = 'raw';
        showRawView();
      }
      updateDirtyState();
    } catch (err) {
      docContent.value = 'Fetch error: ' + err.message;
      docMtime.textContent = '';
      showRawView();
    }
  }

  function isMarkdown(path) {
    return path && path.toLowerCase().endsWith('.md');
  }

  function isYAML(path) {
    if (!path) return false;
    const lower = path.toLowerCase();
    return lower.endsWith('.yaml') || lower.endsWith('.yml');
  }

  function hasRenderedMode(path) {
    return isMarkdown(path) || isYAML(path);
  }

  function showRenderedView() {
    state.viewMode = 'rendered';
    docMode.textContent = 'View: rendered';
    setSplitClass(false);
    docContent.classList.add('hidden');
    docRendered.classList.remove('hidden');
    refreshRenderedFromContent();
  }

  function showSplitView() {
    state.viewMode = 'split';
    docMode.textContent = 'View: split';
    setSplitClass(true);
    docContent.classList.remove('hidden');
    docRendered.classList.remove('hidden');
    refreshRenderedFromContent();
  }

  // refreshRenderedFromContent re-emits the rendered HTML from the current
  // textarea value. Used by both rendered-mode and split-mode (split re-
  // renders on every input event so typing updates the preview live).
  // Per Rain msg 14744 carry-forward: drops the pristine-gate previously
  // applied only to markdown — render uses live docContent.value uniformly
  // across YAML and markdown for consistent mid-edit behavior.
  function refreshRenderedFromContent() {
    if (isYAML(state.currentPath)) {
      docRendered.innerHTML = renderYAML(docContent.value);
    } else if (window.marked) {
      docRendered.innerHTML = renderMarkdownWithTOC(docContent.value);
    } else {
      docRendered.textContent = docContent.value;
    }
  }

  function setSplitClass(on) {
    const docSection = document.querySelector('section.doc');
    if (!docSection) return;
    docSection.classList.toggle('split-mode', !!on);
  }

  // renderMarkdownWithTOC renders markdown via marked.js and prepends a
  // scrollable Table of Contents nav for documents with ≥4 headings (h1-h6).
  // Headings get slug IDs derived from their text content; TOC items link
  // to those anchors. Phase O drain per phase-n.md:820 — addresses the
  // single-big-file navigability gap (e.g., discipline-log.md, phase-n.md).
  //
  // XSS-safe: heading text extracted via textContent (DOM-derived plaintext,
  // no HTML), then escapeHtml'd before injection into TOC anchor labels.
  // Slug derivation uses only \w + hyphens — no HTML-significant chars.
  function renderMarkdownWithTOC(content) {
    const html = window.marked.parse(content);
    const parser = new DOMParser();
    const doc = parser.parseFromString('<div>' + html + '</div>', 'text/html');
    const wrapper = doc.body.firstChild;
    linkifyCiteAnchors(wrapper);
    const headings = wrapper.querySelectorAll('h1, h2, h3, h4, h5, h6');
    if (headings.length < 4) return wrapper.innerHTML;
    const items = [];
    const used = Object.create(null);
    headings.forEach((h) => {
      const text = h.textContent.trim();
      let slug = slugifyHeading(text) || 'section';
      if (used[slug]) {
        const n = used[slug]++;
        slug = slug + '-' + n;
        used[slug] = 1;
      } else {
        used[slug] = 1;
      }
      h.id = slug;
      items.push({ level: parseInt(h.tagName.substring(1), 10), text, slug });
    });
    return buildTOCHtml(items) + wrapper.innerHTML;
  }

  // linkifyCiteAnchors walks the rendered DOM and wraps cite-anchor
  // patterns in <a class="cite-link"> elements that dispatch loadFile()
  // on click. Phase O drain per phase-n.md:817 — addresses the cite-
  // anchor + format-aware link-resolution gap (markdown links already
  // work via marked.js; this layer adds prose-cite navigation:
  // "phase-n.md:820" → click loads phase-n.md). Patterns supported:
  //   <basename>.{md|yaml|yml|json}[:<line>]   → file at canonical-store
  //   <basename>.{md|yaml|yml|json}            → file at canonical-store
  // Skips text inside <code>, <pre>, and existing <a> tags so code blocks
  // and already-linked text aren't re-processed. XSS-safe: text-node
  // matches only; replacement uses createElement + textContent (no
  // innerHTML on user-content paths).
  function linkifyCiteAnchors(root) {
    const skipTags = new Set(['CODE', 'PRE', 'A']);
    const re = /\b([A-Za-z][\w.-]*\.(?:md|yaml|yml|json))(?::(\d+))?\b/g;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
    const targets = [];
    let n;
    while ((n = walker.nextNode())) {
      let p = n.parentNode;
      let skip = false;
      while (p && p !== root) {
        if (skipTags.has(p.tagName)) { skip = true; break; }
        p = p.parentNode;
      }
      if (skip) continue;
      if (re.test(n.nodeValue)) {
        re.lastIndex = 0;
        targets.push(n);
      }
    }
    for (const node of targets) {
      const text = node.nodeValue;
      const frag = document.createDocumentFragment();
      let lastIdx = 0;
      let match;
      re.lastIndex = 0;
      while ((match = re.exec(text)) !== null) {
        if (match.index > lastIdx) {
          frag.appendChild(document.createTextNode(text.substring(lastIdx, match.index)));
        }
        const a = document.createElement('a');
        a.className = 'cite-link';
        a.href = '#';
        a.dataset.citePath = match[1];
        if (match[2]) a.dataset.citeLine = match[2];
        a.textContent = match[0];
        a.title = 'Open ' + match[1] + (match[2] ? ' (line ' + match[2] + ')' : '');
        a.addEventListener('click', (ev) => {
          ev.preventDefault();
          resolveCiteLink(a.dataset.citePath);
        });
        frag.appendChild(a);
        lastIdx = re.lastIndex;
      }
      if (lastIdx < text.length) {
        frag.appendChild(document.createTextNode(text.substring(lastIdx)));
      }
      node.parentNode.replaceChild(frag, node);
    }
  }

  // resolveCiteLink dispatches a cite-anchor click. For an unqualified
  // basename, we look up the canonical-store path via the destinations
  // already loaded for the active project + global. If a unique match is
  // found, loadFile fires; on ambiguity or miss, status surfaces inline.
  // Phase O scope: best-effort lookup using cached destination tree; no
  // backend resolver call — keeps this client-side simple.
  function resolveCiteLink(basename) {
    const candidates = [];
    document.querySelectorAll('.nav-list .file-link[data-path]').forEach((el) => {
      const p = el.dataset.path || '';
      const tail = p.split('/').pop();
      if (tail === basename) candidates.push(p);
    });
    if (candidates.length === 1) {
      loadFile(candidates[0]);
      return;
    }
    if (candidates.length > 1) {
      docStatus.textContent = 'Cite "' + basename + '" matches ' + candidates.length + ' files; pick from nav.';
      return;
    }
    docStatus.textContent = 'Cite "' + basename + '" not found in current nav (load destinations first).';
  }

  function slugifyHeading(text) {
    return String(text)
      .toLowerCase()
      .replace(/[^\w\s-]+/g, '')
      .trim()
      .replace(/\s+/g, '-')
      .replace(/-+/g, '-')
      .replace(/^-|-$/g, '');
  }

  function buildTOCHtml(items) {
    const parts = ['<nav class="md-toc"><div class="md-toc-title">Table of contents</div><ul>'];
    for (const it of items) {
      parts.push(
        '<li class="toc-l' + it.level + '"><a href="#' + it.slug + '">' +
        escapeHtml(it.text) + '</a></li>'
      );
    }
    parts.push('</ul></nav>');
    return parts.join('');
  }

  // renderYAML produces a syntax-highlighted read-only HTML view of YAML
  // content using a regex-based per-line tokenizer. No external library —
  // covers the common YAML constructs (keys, quoted strings, comments,
  // bool/null literals, numbers, list markers, block-scalar markers).
  // Multi-line edge cases (anchors, aliases, complex flow mappings) fall
  // back to plain escaped text — future polish if a use case surfaces.
  function renderYAML(content) {
    const html = String(content)
      .split('\n')
      .map(highlightYAMLLine)
      .join('\n');
    return '<pre class="yaml-rendered">' + html + '</pre>';
  }

  function highlightYAMLLine(rawLine) {
    // Step 1: scan for first unquoted '#' to split body from comment.
    let inSingle = false, inDouble = false, commentStart = -1;
    for (let i = 0; i < rawLine.length; i++) {
      const c = rawLine[i];
      if (inSingle) { if (c === "'") inSingle = false; }
      else if (inDouble) { if (c === '"' && rawLine[i - 1] !== '\\') inDouble = false; }
      else if (c === "'") inSingle = true;
      else if (c === '"') inDouble = true;
      else if (c === '#' && (i === 0 || /\s/.test(rawLine[i - 1]))) { commentStart = i; break; }
    }
    const body = commentStart >= 0 ? rawLine.substring(0, commentStart) : rawLine;
    const comment = commentStart >= 0 ? rawLine.substring(commentStart) : '';
    let out = escapeHtml(body)
      .replace(/^(\s*)(-\s+)?([A-Za-z_][\w.-]*)(\s*:)/,
        (m, indent, dash, key, colon) =>
          indent + (dash ? '<span class="yaml-marker">' + dash + '</span>' : '') +
          '<span class="yaml-key">' + key + '</span><span class="yaml-colon">' + colon + '</span>')
      .replace(/("[^"]*"|'[^']*')/g, '<span class="yaml-string">$1</span>')
      .replace(/(:\s)(true|false|null|yes|no)(\s|$)/g,
        '$1<span class="yaml-bool">$2</span>$3')
      .replace(/(:\s)(-?\d+(?:\.\d+)?)(\s|$)/g,
        '$1<span class="yaml-number">$2</span>$3')
      .replace(/(:\s)([|]|&gt;)([-+]?)(\s|$)/g,
        '$1<span class="yaml-scalar-marker">$2$3</span>$4');
    if (comment) out += '<span class="yaml-comment">' + escapeHtml(comment) + '</span>';
    return out;
  }

  function showRawView() {
    state.viewMode = 'raw';
    docMode.textContent = 'View: raw';
    setSplitClass(false);
    docRendered.classList.add('hidden');
    docContent.classList.remove('hidden');
  }

  function toggleViewMode() {
    if (!state.currentPath) return;
    if (!hasRenderedMode(state.currentPath)) {
      // No rendered mode for this file type; raw only.
      showRawView();
      return;
    }
    // Three-mode cycle for renderable files: rendered → split → raw → rendered.
    if (state.viewMode === 'rendered') showSplitView();
    else if (state.viewMode === 'split') showRawView();
    else showRenderedView();
  }

  function onEdit() {
    updateDirtyState();
    // Split-mode renders live: typing updates preview pane on every input.
    // Rendered-mode (preview-only) leaves rendered HTML alone — refreshes
    // on next view-toggle / save (typing isn't visible in rendered-only).
    if (state.viewMode === 'split') {
      refreshRenderedFromContent();
    }
  }

  function isDirty() {
    return state.currentPath != null && docContent.value !== state.pristine;
  }

  function updateDirtyState() {
    const dirty = isDirty();
    docDirty.classList.toggle('hidden', !dirty);
    docSave.disabled = !dirty;
  }

  async function saveFile() {
    if (!state.currentPath) return;
    docStatus.textContent = 'Saving…';
    docSave.disabled = true;
    try {
      const res = await fetch('/api/files/' + state.currentPath, {
        method: 'POST',
        headers: {
          'If-Match': state.currentMtime || '',
          'Content-Type': 'text/plain; charset=utf-8',
        },
        body: docContent.value,
      });
      if (res.status === 409) {
        const payload = await res.json();
        state.pendingConflict = payload;
        conflictServer.textContent = payload.current_content || '';
        conflictModal.classList.remove('hidden');
        docStatus.textContent = '';
        return;
      }
      if (!res.ok) {
        const text = await res.text();
        docStatus.textContent = 'Save failed: ' + text;
        updateDirtyState();
        return;
      }
      const data = await res.json();
      state.currentMtime = data.mtime || '';
      state.pristine = docContent.value;
      docMtime.textContent = data.mtime || '';
      const sha = data.commit ? ' · commit ' + data.commit.slice(0, 7) : '';
      const warns = (data.warnings && data.warnings.length) ? ' · ' + data.warnings.length + ' warning(s)' : '';
      docStatus.textContent = 'Saved ' + (data.mtime || '') + sha + warns;
      updateDirtyState();
      // Refresh nav + recent-edits feed to pick up new mtimes.
      loadDestinations();
      loadRecentEdits();
      // If rendered view active, refresh render with new content.
      if (state.viewMode === 'rendered' && isMarkdown(state.currentPath)) {
        showRenderedView();
      }
    } catch (err) {
      docStatus.textContent = 'Save error: ' + err.message;
      updateDirtyState();
    }
  }

  async function resolveConflict(action) {
    const conflict = state.pendingConflict;
    conflictModal.classList.add('hidden');
    if (!conflict) return;
    if (action === 'discard') {
      state.pristine = conflict.current_content || '';
      state.currentMtime = conflict.current_mtime || '';
      docContent.value = state.pristine;
      docMtime.textContent = state.currentMtime;
      docStatus.textContent = 'Discarded local edits; loaded server version.';
      if (state.viewMode === 'rendered' && isMarkdown(state.currentPath)) showRenderedView();
      updateDirtyState();
    } else if (action === 'overwrite') {
      state.currentMtime = conflict.current_mtime || '';
      state.pendingConflict = null;
      await saveFile();
    } else {
      docStatus.textContent = 'Keeping local edits; server has newer version (' + (conflict.current_mtime || '') + ').';
    }
    state.pendingConflict = null;
  }

  // Cross-search dashboard per phase-n.md:819. Modal-based content
  // search across the canonical-store. Debounced 200ms input → GET
  // /api/search?q=...&limit=30 → renders results list with path +
  // line + snippet; click loads the matching file. Ctrl+K (Cmd+K on
  // mac) opens modal; Escape closes. XSS-clean: snippet rendered via
  // textContent (server-side never escapes, client-side never
  // innerHTMLs user content).
  let searchDebounce = null;

  function openSearchModal() {
    document.getElementById('search-modal').classList.remove('hidden');
    const input = document.getElementById('search-input');
    input.focus();
    input.select();
  }

  function closeSearchModal() {
    document.getElementById('search-modal').classList.add('hidden');
  }

  function onSearchInput() {
    if (searchDebounce !== null) clearTimeout(searchDebounce);
    searchDebounce = setTimeout(runSearch, 200);
  }

  async function runSearch() {
    const q = document.getElementById('search-input').value.trim();
    const out = document.getElementById('search-results');
    if (q.length < 2) {
      out.innerHTML = '<em class="muted">Type 2+ chars to search.</em>';
      return;
    }
    out.innerHTML = '<em class="muted">Searching…</em>';
    try {
      const res = await fetch('/api/search?q=' + encodeURIComponent(q) + '&limit=30');
      if (!res.ok) {
        out.innerHTML = '<em class="error">Search failed.</em>';
        return;
      }
      const data = await res.json();
      const results = data.results || [];
      if (results.length === 0) {
        out.innerHTML = '<em class="muted">No matches.</em>';
        return;
      }
      out.innerHTML = '';
      const ul = document.createElement('ul');
      ul.className = 'search-list';
      for (const r of results) {
        const li = document.createElement('li');
        const head = document.createElement('div');
        head.className = 'search-head';
        const a = document.createElement('a');
        a.href = '#';
        a.className = 'search-path';
        a.textContent = r.path + ':' + r.line;
        a.title = r.path;
        a.addEventListener('click', (ev) => {
          ev.preventDefault();
          closeSearchModal();
          loadFile(r.path);
        });
        head.appendChild(a);
        li.appendChild(head);
        const snip = document.createElement('div');
        snip.className = 'search-snippet';
        snip.textContent = r.snippet || '';
        li.appendChild(snip);
        ul.appendChild(li);
      }
      out.appendChild(ul);
    } catch (err) {
      out.innerHTML = '<em class="error">Network error.</em>';
    }
  }

  // Recent-edits feed widget per phase-n.md:816. Renders the top-10
  // most-recently-modified canonical-store files in the sidebar with
  // relative-time labels. Click loads the file via the existing loadFile
  // dispatch. Refreshed on init + after every successful save.
  async function loadRecentEdits() {
    if (!navRecent) return;
    try {
      const res = await fetch('/api/recent-edits?limit=10');
      const data = await res.json();
      const edits = data.edits || [];
      if (edits.length === 0) {
        navRecent.innerHTML = '<em class="muted">No edits yet.</em>';
        return;
      }
      navRecent.innerHTML = '';
      const ul = document.createElement('ul');
      ul.className = 'recent-list';
      for (const e of edits) {
        const li = document.createElement('li');
        const a = document.createElement('a');
        a.href = '#';
        a.className = 'recent-link';
        a.dataset.path = e.path;
        a.textContent = e.name;
        a.title = e.path;
        a.addEventListener('click', (ev) => {
          ev.preventDefault();
          loadFile(e.path);
        });
        const ts = document.createElement('span');
        ts.className = 'recent-time muted';
        ts.textContent = formatRelativeTime(e.mtime);
        li.appendChild(a);
        li.appendChild(ts);
        ul.appendChild(li);
      }
      navRecent.appendChild(ul);
    } catch (err) {
      navRecent.innerHTML = '<em class="error">Failed to load recent edits.</em>';
    }
  }

  // formatRelativeTime returns a short "3m ago" / "2h ago" / "1d ago"
  // / "Mar 5" label given an ISO 8601 UTC timestamp string. Falls back
  // to the raw string on parse failure.
  function formatRelativeTime(iso) {
    const t = Date.parse(iso);
    if (Number.isNaN(t)) return iso;
    const diff = (Date.now() - t) / 1000;
    if (diff < 60) return 'just now';
    if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
    if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
    if (diff < 7 * 86400) return Math.floor(diff / 86400) + 'd ago';
    const d = new Date(t);
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }

  // Register-project formal flow per phase-n.md:826. POST /api/projects
  // creates ~/.bot-hq/projects/<name>.yaml from a starter template; on
  // success the project list is refreshed and the new project is selected.
  function openRegisterModal() {
    document.getElementById('register-project-name').value = '';
    document.getElementById('register-project-remote').value = '';
    document.getElementById('register-project-status').textContent = '';
    document.getElementById('register-project-modal').classList.remove('hidden');
    document.getElementById('register-project-name').focus();
  }

  function closeRegisterModal() {
    document.getElementById('register-project-modal').classList.add('hidden');
  }

  async function submitRegisterProject() {
    const name = document.getElementById('register-project-name').value.trim();
    const remote = document.getElementById('register-project-remote').value.trim();
    const statusEl = document.getElementById('register-project-status');
    if (!/^[a-z][a-z0-9-]{1,63}$/.test(name)) {
      statusEl.textContent = 'Invalid name. Use lowercase letters/digits/hyphens, 2-64 chars, starting with a letter.';
      return;
    }
    statusEl.textContent = 'Registering…';
    try {
      const res = await fetch('/api/projects', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name, remote_url: remote }),
      });
      const text = await res.text();
      if (!res.ok) {
        statusEl.textContent = 'Failed: ' + text;
        return;
      }
      statusEl.textContent = 'Registered. Switching to ' + name + '…';
      await loadProjects();
      projectPicker.value = name;
      state.project = name;
      activeChip.textContent = name;
      await loadDestinations();
      closeRegisterModal();
    } catch (err) {
      statusEl.textContent = 'Network error: ' + err.message;
    }
  }

  function escapeHtml(s) {
    return String(s)
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;');
  }
})();
