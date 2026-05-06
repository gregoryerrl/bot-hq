// bot-hq Clive workspace — Phase N v3b/v3c frontend.
// Vanilla JS + fetch. Read MVP shipped v3b; v3c adds editor + Save +
// 409-conflict UX + Clive proposal flow (proposal acceptance UI deferred).

(function () {
  'use strict';

  const tree = document.getElementById('tree');
  const docPath = document.getElementById('doc-path');
  const docMtime = document.getElementById('doc-mtime');
  const docContent = document.getElementById('doc-content');
  const docDirty = document.getElementById('doc-dirty');
  const docSave = document.getElementById('doc-save');
  const docStatus = document.getElementById('doc-status');
  const rulesPre = document.getElementById('rules');
  const clive = document.getElementById('clive');
  const conflictModal = document.getElementById('conflict-modal');
  const conflictServer = document.getElementById('conflict-server-content');

  // Editor state
  const state = {
    currentPath: null,
    currentMtime: null,
    pristine: '',
    pendingConflict: null, // { current_mtime, current_content }
  };

  document.getElementById('tree-refresh').addEventListener('click', loadTree);
  document.getElementById('rules-refresh').addEventListener('click', loadRules);
  document.getElementById('clive-refresh').addEventListener('click', loadClive);
  docContent.addEventListener('input', onEdit);
  docSave.addEventListener('click', saveFile);
  document.getElementById('conflict-overwrite').addEventListener('click', resolveConflict.bind(null, 'overwrite'));
  document.getElementById('conflict-discard').addEventListener('click', resolveConflict.bind(null, 'discard'));
  document.getElementById('conflict-keep').addEventListener('click', resolveConflict.bind(null, 'keep'));

  loadTree();
  loadClive();

  async function loadTree() {
    tree.innerHTML = '<em>Loading…</em>';
    try {
      const res = await fetch('/api/files');
      const data = await res.json();
      tree.innerHTML = '';
      tree.appendChild(renderTree(data.tree || []));
    } catch (err) {
      tree.innerHTML = '<em class="error">Failed to load tree: ' + escapeHtml(err.message) + '</em>';
    }
  }

  function renderTree(nodes) {
    const ul = document.createElement('ul');
    for (const node of nodes) {
      const li = document.createElement('li');
      li.classList.add(node.type);
      if (node.type === 'dir') {
        const span = document.createElement('span');
        span.classList.add('dir-name');
        span.textContent = node.name + '/';
        li.appendChild(span);
        li.appendChild(renderTree(node.children || []));
      } else {
        const a = document.createElement('a');
        a.href = '#' + node.path;
        a.textContent = node.name;
        a.title = (node.size != null ? node.size + ' B' : '') + ' · ' + (node.mtime || '');
        a.addEventListener('click', (ev) => {
          ev.preventDefault();
          if (state.currentPath && isDirty()) {
            if (!confirm('You have unsaved changes. Discard and load new file?')) return;
          }
          loadFile(node.path);
        });
        li.appendChild(a);
      }
      ul.appendChild(li);
    }
    return ul;
  }

  async function loadFile(path) {
    docPath.textContent = path;
    docMtime.textContent = 'loading…';
    docContent.value = '';
    docContent.disabled = true;
    docSave.disabled = true;
    docDirty.classList.add('hidden');
    docStatus.textContent = '';
    try {
      const res = await fetch('/api/files/' + path + '?format=json');
      if (!res.ok) {
        docContent.value = 'Error: ' + res.status + ' ' + res.statusText;
        docMtime.textContent = '';
        return;
      }
      const data = await res.json();
      state.currentPath = path;
      state.currentMtime = data.mtime || '';
      state.pristine = data.content || '';
      docContent.value = state.pristine;
      docContent.disabled = false;
      docMtime.textContent = data.mtime || '';
      updateDirtyState();
    } catch (err) {
      docContent.value = 'Fetch error: ' + err.message;
      docMtime.textContent = '';
    }
  }

  function onEdit() {
    updateDirtyState();
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
      // Refresh tree mtimes (fire-and-forget).
      loadTree();
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
      // Replace local edits with server's current content.
      state.pristine = conflict.current_content || '';
      state.currentMtime = conflict.current_mtime || '';
      docContent.value = state.pristine;
      docMtime.textContent = state.currentMtime;
      docStatus.textContent = 'Discarded local edits; loaded server version.';
      updateDirtyState();
    } else if (action === 'overwrite') {
      // Force overwrite by retrying with the server's current mtime.
      state.currentMtime = conflict.current_mtime || '';
      state.pendingConflict = null;
      await saveFile();
    } else {
      // 'keep' — leave editor alone with local edits + new server mtime
      // hidden so user can manually reconcile.
      docStatus.textContent = 'Keeping local edits; server has newer version (' + (conflict.current_mtime || '') + ').';
    }
    state.pendingConflict = null;
  }

  async function loadRules() {
    const project = document.getElementById('rules-project').value.trim();
    const agent = document.getElementById('rules-agent').value.trim();
    const params = new URLSearchParams();
    if (project) params.set('project', project);
    if (agent) params.set('agent', agent);
    rulesPre.textContent = 'loading…';
    try {
      const res = await fetch('/api/rules?' + params.toString());
      const data = await res.json();
      rulesPre.textContent = JSON.stringify(data, null, 2);
    } catch (err) {
      rulesPre.textContent = 'Failed: ' + err.message;
    }
  }

  async function loadClive() {
    clive.innerHTML = '<em>loading…</em>';
    try {
      const res = await fetch('/api/clive/activity');
      const data = await res.json();
      const messages = data.messages || [];
      if (!messages.length) {
        clive.innerHTML = '<em>No Clive messages yet.</em>';
        return;
      }
      const ul = document.createElement('ul');
      ul.classList.add('clive-list');
      for (const m of messages) {
        const li = document.createElement('li');
        const ts = document.createElement('span');
        ts.classList.add('ts');
        ts.textContent = m.created || '';
        const body = document.createElement('span');
        body.classList.add('body');
        body.textContent = '[' + m.type + ' → ' + (m.to_agent || 'broadcast') + '] ' + m.content;
        li.appendChild(ts);
        li.appendChild(body);
        ul.appendChild(li);
      }
      clive.innerHTML = '';
      clive.appendChild(ul);
    } catch (err) {
      clive.innerHTML = '<em class="error">Failed: ' + escapeHtml(err.message) + '</em>';
    }
  }

  function escapeHtml(s) {
    return String(s)
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;');
  }
})();
