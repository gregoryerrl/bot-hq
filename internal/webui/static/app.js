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

  loadProjects().then(loadDestinations);

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
    const headings = wrapper.querySelectorAll('h1, h2, h3, h4, h5, h6');
    if (headings.length < 4) return html;
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
      // Refresh nav to pick up new mtimes.
      loadDestinations();
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
