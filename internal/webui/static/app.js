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
      if (isMarkdown(path) && state.viewMode === 'rendered') {
        showRenderedView();
      } else {
        // Non-md always raw; md follows current viewMode.
        if (!isMarkdown(path)) state.viewMode = 'raw';
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

  function showRenderedView() {
    state.viewMode = 'rendered';
    docMode.textContent = 'View: rendered';
    docContent.classList.add('hidden');
    docRendered.classList.remove('hidden');
    if (window.marked && state.pristine) {
      docRendered.innerHTML = window.marked.parse(docContent.value);
    } else {
      docRendered.textContent = docContent.value;
    }
  }

  function showRawView() {
    state.viewMode = 'raw';
    docMode.textContent = 'View: raw';
    docRendered.classList.add('hidden');
    docContent.classList.remove('hidden');
  }

  function toggleViewMode() {
    if (!state.currentPath) return;
    if (!isMarkdown(state.currentPath)) {
      // Non-md only has raw view.
      showRawView();
      return;
    }
    if (state.viewMode === 'rendered') {
      showRawView();
    } else {
      showRenderedView();
    }
  }

  function onEdit() {
    updateDirtyState();
    // If currently rendered, leave rendered alone — render refreshes on
    // next view-toggle / save.
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

  function escapeHtml(s) {
    return String(s)
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;');
  }
})();
