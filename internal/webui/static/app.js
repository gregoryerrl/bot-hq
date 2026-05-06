// bot-hq Clive workspace — Phase N v3b read MVP frontend.
// Vanilla JS + fetch; no framework dependencies. Manual F5 to refresh
// (no auto-refresh per Q4 LOCKED). Write capability + Clive integration
// + raw-YAML rules editor land in v3c.

(function () {
  'use strict';

  const tree = document.getElementById('tree');
  const docPath = document.getElementById('doc-path');
  const docMtime = document.getElementById('doc-mtime');
  const docContent = document.getElementById('doc-content');
  const rulesPre = document.getElementById('rules');
  const clive = document.getElementById('clive');

  document.getElementById('rules-refresh').addEventListener('click', loadRules);
  document.getElementById('clive-refresh').addEventListener('click', loadClive);

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
    docContent.textContent = '';
    try {
      const res = await fetch('/api/files/' + path + '?format=json');
      if (!res.ok) {
        docContent.textContent = 'Error: ' + res.status + ' ' + res.statusText;
        docMtime.textContent = '';
        return;
      }
      const data = await res.json();
      docMtime.textContent = data.mtime || '';
      docContent.textContent = data.content || '';
    } catch (err) {
      docContent.textContent = 'Fetch error: ' + err.message;
      docMtime.textContent = '';
    }
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
