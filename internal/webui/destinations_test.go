package webui

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fillCanonicalLayout creates a fixture canonical-store with the curated
// surfaces (and several HIDE-list runtime files) that v3.x-1 destinations
// must surface (or hide).
func fillCanonicalLayout(t *testing.T, root string) {
	t.Helper()
	mustWrite(t, filepath.Join(root, "README.md"), "# bot-hq\n")
	mustWrite(t, filepath.Join(root, "tasks.md"), "---\ntasks: []\n---\n\n# Global tasks\n")
	// Post-Z-1: bot-hq's operational artifacts under projects/bot-hq/
	mustMkdir(t, filepath.Join(root, "projects", "bot-hq"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "discipline-log.md"), "# Discipline log\n")

	mustMkdir(t, filepath.Join(root, "projects", "bot-hq", "phase"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "phase", "phase-n.md"), "# Phase N\n")
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "phase", "phase-i.md"), "# Phase I\n")

	mustMkdir(t, filepath.Join(root, "projects", "bot-hq", "ratchets"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "ratchets", "active.md"), "ratchet\n")

	mustMkdir(t, filepath.Join(root, "rules"))
	mustWrite(t, filepath.Join(root, "rules", "general.yaml"), "tone:\n  reply: g\n")

	mustMkdir(t, filepath.Join(root, "rules", "agents"))
	mustWrite(t, filepath.Join(root, "rules", "agents", "brian.yaml"), "role: HANDS\n")
	mustWrite(t, filepath.Join(root, "rules", "agents", "rain.yaml"), "role: EYES\n")

	mustMkdir(t, filepath.Join(root, "brian"))
	mustWrite(t, filepath.Join(root, "brian", "discipline-anchors.md"), "anchors\n")
	mustWrite(t, filepath.Join(root, "brian", "last_state.json"), "{}\n") // HIDE
	mustMkdir(t, filepath.Join(root, "rain"))
	mustWrite(t, filepath.Join(root, "rain", "discipline-anchors.md"), "rain anchors\n")

	mustMkdir(t, filepath.Join(root, "gates"))
	mustWrite(t, filepath.Join(root, "gates", "pre-commit-checklist.md"), "gate\n")
	mustWrite(t, filepath.Join(root, "gates", "pre-push-checklist.md"), "gate\n")

	mustMkdir(t, filepath.Join(root, "sessions", "2026-05-06-session"))
	mustWrite(t, filepath.Join(root, "sessions", "2026-05-06-session", "manifest.md"), "manifest\n")

	mustMkdir(t, filepath.Join(root, "projects"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq.yaml"), "project_name: bot-hq\n")
	mustWrite(t, filepath.Join(root, "projects", "myproj.yaml"), "project_name: myproj\n")
	// Phase Q library schema fixture for myproj. README.md + INDEX.md are
	// the new Overview surface (replaces v3.x-1 overview.md). Each library
	// subdir gets 1 fixture file so destination resolvers have content to
	// report.
	mustMkdir(t, filepath.Join(root, "projects", "myproj"))
	mustWrite(t, filepath.Join(root, "projects", "myproj", "README.md"), "# myproj\n")
	mustWrite(t, filepath.Join(root, "projects", "myproj", "INDEX.md"), "# myproj index\n")
	for _, sub := range []string{"plans", "clips", "eod", "architecture", "decisions", "conventions", "glossary", "audit-notes"} {
		mustMkdir(t, filepath.Join(root, "projects", "myproj", sub))
		mustWrite(t, filepath.Join(root, "projects", "myproj", sub, sub+"-1.md"), sub+"\n")
	}
	// bot-hq library README/INDEX — Phase Q convention applies
	// symmetrically (bot-hq is just-another-project).
	mustMkdir(t, filepath.Join(root, "projects", "bot-hq"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "README.md"), "# bot-hq library\n")
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "INDEX.md"), "# bot-hq index\n")

	// HIDE-list noise that should never surface.
	mustWrite(t, filepath.Join(root, "hub.db"), "binary")
	mustWrite(t, filepath.Join(root, "live.log"), "log\n")
	mustWrite(t, filepath.Join(root, "voice-mirror-log.md"), "voice\n")
	mustMkdir(t, filepath.Join(root, "diag"))
	mustWrite(t, filepath.Join(root, "diag", "dedup.jsonl"), "{}\n")
	mustMkdir(t, filepath.Join(root, "sentinels"))
	mustMkdir(t, filepath.Join(root, "bridge"))
	mustMkdir(t, filepath.Join(root, "plugins"))
	mustMkdir(t, filepath.Join(root, "plugins", "github"))
	mustWrite(t, filepath.Join(root, "plugins", "github", "index.ts"), "// source\n")
}

func TestListProjects_DiscoversFromYaml(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)

	projects, err := ListProjects(root)
	if err != nil {
		t.Fatalf("ListProjects: %v", err)
	}
	if len(projects) < 2 {
		t.Fatalf("expected at least 2 projects, got %d", len(projects))
	}
	if projects[0].Name != "bot-hq" {
		t.Errorf("first project = %q, want bot-hq", projects[0].Name)
	}
	found := false
	for _, p := range projects {
		if p.Name == "myproj" {
			found = true
		}
	}
	if !found {
		t.Errorf("expected myproj in projects, got %v", projects)
	}
}

func TestListProjects_EmptyDirOnlyBotHQ(t *testing.T) {
	root := t.TempDir() // no projects/ dir
	projects, err := ListProjects(root)
	if err != nil {
		t.Fatalf("ListProjects: %v", err)
	}
	if len(projects) != 1 || projects[0].Name != "bot-hq" {
		t.Errorf("expected only bot-hq, got %v", projects)
	}
}

// TestResolveDestinations_GlobalSection_BotHQProject + sibling
// TestResolveDestinations_{BotHQProjectSection,NonBotHQProjectSection,
// OverviewBlankState,HideListEnforced,EmptyCanonicalStore} tested the
// pre-Phase-R3-R5 hardcoded resolver list (25+ resolveProject* / global
// resolvers in destinations.go). Phase R3 R5 S4 dropped the resolver
// list; coverage migrated to treewalker_test.go + crossproject_test.go
// for the yaml-driven nav model that replaced it.

func TestHandleProjectsEndpoint(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/projects")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	var p struct {
		Projects []Project `json:"projects"`
	}
	if err := json.Unmarshal([]byte(body), &p); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(p.Projects) < 2 || p.Projects[0].Name != "bot-hq" {
		t.Errorf("projects = %+v", p.Projects)
	}
}

// TestHandleDestinationsEndpoint covers the post-Phase-R3-R5 backward-
// compat shim: the route stays alive returning an empty Destination list
// until S5 atomic-deletion + frontend migration to /api/files?tree=1.
func TestHandleDestinationsEndpoint(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)
	s := newTestServer(t, root)

	status, body := callRoute(t, s, "GET", "/api/destinations?project=bot-hq")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	var p struct {
		Project      string        `json:"project"`
		Destinations []Destination `json:"destinations"`
	}
	if err := json.Unmarshal([]byte(body), &p); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if p.Project != "bot-hq" {
		t.Errorf("project = %q, want bot-hq", p.Project)
	}
	// Post-S4: destinations list is empty until S5 deletes the route.
	if len(p.Destinations) != 0 {
		t.Errorf("destinations count = %d, want 0 (backward-compat shim); got %+v", len(p.Destinations), p.Destinations)
	}
}

func TestHandleDestinationsEndpoint_DefaultBotHQ(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)
	s := newTestServer(t, root)
	// no project query param — should default to bot-hq
	status, body := callRoute(t, s, "GET", "/api/destinations")
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	if !strings.Contains(body, `"project": "bot-hq"`) {
		t.Errorf("missing default bot-hq project; body=%s", body[:min(300, len(body))])
	}
}

// HIDE-list verification — confirm runtime/code paths absent from any
// destination output. Walks every destination's Nodes recursively.
// TestResolveDestinations_HideListEnforced previously walked the resolver
// output to assert no HIDE-class path surfaced; that responsibility moved
// to cl.IsHidden (covered by internal/cl/hidden_test.go) and the
// tree-walker (covered by treewalker_test.go).

// HIDE-list at file-content endpoint level: even if a path is requested,
// it must be rejected if it's HIDE-class.
func TestHandleFileContent_RejectsHideListExtension(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "live.log"), "log\n")
	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "GET", "/api/files/live.log")
	if status == http.StatusOK {
		t.Errorf("expected non-200 for HIDE-extension .log path, got %d", status)
	}
}

func TestHandleFileContent_RejectsVoiceMirrorLog(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "voice-mirror-log.md"), "voice\n")
	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "GET", "/api/files/voice-mirror-log.md")
	if status == http.StatusOK {
		t.Errorf("expected non-200 for voice-mirror-log.md, got %d", status)
	}
}

func TestHandleFileContent_AllowsAgentDisciplineAnchors(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "brian"))
	mustWrite(t, filepath.Join(root, "brian", "discipline-anchors.md"), "anchors\n")

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/brian/discipline-anchors.md")
	if status != http.StatusOK {
		t.Errorf("status = %d, want 200; body=%s", status, body)
	}
}

func TestHandleFileContent_RejectsAgentLastStateJson(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "brian"))
	mustWrite(t, filepath.Join(root, "brian", "last_state.json"), "{}\n")

	s := newTestServer(t, root)
	status, _ := callRoute(t, s, "GET", "/api/files/brian/last_state.json")
	if status == http.StatusOK {
		t.Errorf("expected non-200 for last_state.json (HIDE), got %d", status)
	}
}

func TestHandleFileContent_AllowsGatesMd(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "gates"))
	mustWrite(t, filepath.Join(root, "gates", "pre-commit-checklist.md"), "gate\n")
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/gates/pre-commit-checklist.md")
	if status != http.StatusOK {
		t.Errorf("status = %d, want 200 (gates allowlisted); body=%s", status, body)
	}
}

func TestHandleFileContent_AllowsSessionManifest(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "sessions", "2026-05-06-foo"))
	mustWrite(t, filepath.Join(root, "sessions", "2026-05-06-foo", "manifest.md"), "manifest\n")
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/files/sessions/2026-05-06-foo/manifest.md")
	if status != http.StatusOK {
		t.Errorf("status = %d, want 200 (session manifest allowlisted); body=%s", status, body)
	}
}

// Path-escape defense — preserved through v3.x-1 refactor.
func TestResolveCanonicalPath_StillRejectsEscape_v3x1(t *testing.T) {
	root := t.TempDir()
	if _, err := resolveCanonicalPath(root, "../etc/passwd"); err == nil {
		t.Errorf("expected error for ../etc/passwd")
	}
	if _, err := resolveCanonicalPath(root, "phase/../../etc/passwd"); err == nil {
		t.Errorf("expected error for nested escape")
	}
}

// Sanity: a brand-new install (no canonical-store dirs at all) should
// not crash the backward-compat shim. Post-S4 returns an empty list.
func TestResolveDestinations_EmptyCanonicalStore(t *testing.T) {
	root := t.TempDir()
	dests, err := ResolveDestinations(root, "bot-hq")
	if err != nil {
		t.Fatalf("%v", err)
	}
	if len(dests) != 0 {
		t.Errorf("expected 0 destinations (S4 backward-compat shim), got %d", len(dests))
	}
}

// Sanity: the JSON marshalling round-trips with our struct shape.
func TestDestination_JSONShape(t *testing.T) {
	d := Destination{Name: "X", Section: "global", Nodes: []TreeNode{{Name: "a", Path: "a"}}}
	b, err := json.Marshal(d)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(b), `"name":"X"`) || !strings.Contains(string(b), `"section":"global"`) {
		t.Errorf("unexpected JSON: %s", b)
	}
	// Resolver should NOT serialize.
	if strings.Contains(string(b), "Resolver") {
		t.Errorf("Resolver leaked into JSON: %s", b)
	}
}

// Auto-start lifecycle smoke — ensures the env-disable opt-out is read
// correctly. We can't easily test the goroutine spawn without invoking
// runHub, but we can sanity-check the env-flag check is correct by
// constructing the Server directly.
func TestNewServer_ConstructsCleanly(t *testing.T) {
	old := os.Getenv("BOT_HQ_WEBUI_DISABLE")
	defer os.Setenv("BOT_HQ_WEBUI_DISABLE", old)
	os.Unsetenv("BOT_HQ_WEBUI_DISABLE")

	srv, err := NewServer(nil, WithRoot(t.TempDir()), WithPort(0))
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	if srv == nil {
		t.Fatal("nil server")
	}
}
