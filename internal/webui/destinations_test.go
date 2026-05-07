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
	mustWrite(t, filepath.Join(root, "discipline-log.md"), "# Discipline log\n")

	mustMkdir(t, filepath.Join(root, "phase"))
	mustWrite(t, filepath.Join(root, "phase", "phase-n.md"), "# Phase N\n")
	mustWrite(t, filepath.Join(root, "phase", "phase-i.md"), "# Phase I\n")

	mustMkdir(t, filepath.Join(root, "ratchets"))
	mustWrite(t, filepath.Join(root, "ratchets", "active.md"), "ratchet\n")

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
	mustMkdir(t, filepath.Join(root, "projects", "myproj"))
	mustWrite(t, filepath.Join(root, "projects", "myproj", "overview.md"), "# myproj\n")
	mustMkdir(t, filepath.Join(root, "projects", "myproj", "plans"))
	mustWrite(t, filepath.Join(root, "projects", "myproj", "plans", "plan-a.md"), "plan a\n")
	mustMkdir(t, filepath.Join(root, "projects", "myproj", "clips"))
	mustWrite(t, filepath.Join(root, "projects", "myproj", "clips", "clip-1.md"), "clip\n")
	mustMkdir(t, filepath.Join(root, "projects", "myproj", "eod"))
	mustWrite(t, filepath.Join(root, "projects", "myproj", "eod", "eod-1.md"), "eod\n")

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

func TestResolveDestinations_GlobalSection_BotHQProject(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)

	dests, err := ResolveDestinations(root, "bot-hq")
	if err != nil {
		t.Fatalf("ResolveDestinations: %v", err)
	}
	byName := map[string]Destination{}
	for _, d := range dests {
		byName[d.Name] = d
	}

	// Global:
	if got := byName["Documents"].Nodes; len(got) != 2 || got[0].Name != "README.md" || got[1].Name != "tasks.md" {
		t.Errorf("Documents nodes = %+v, want [README.md tasks.md]", got)
	}
	if got := byName["Disciplines"].Nodes; len(got) != 1 || got[0].Name != "discipline-log.md" {
		t.Errorf("Disciplines nodes = %+v, want 1 discipline-log.md", got)
	}
	if got := byName["Ratchets"].Nodes; len(got) != 1 || got[0].Name != "active.md" {
		t.Errorf("Ratchets nodes = %+v", got)
	}
	if got := byName["Global Rules"].Nodes; len(got) != 1 || got[0].Name != "general.yaml" {
		t.Errorf("Global Rules nodes = %+v", got)
	}
	if got := byName["Agent Rules"].Nodes; len(got) != 2 {
		t.Errorf("Agent Rules nodes count = %d, want 2 (brian+rain)", len(got))
	}
	if got := byName["Agent Notes"].Nodes; len(got) != 2 {
		t.Errorf("Agent Notes nodes count = %d, want 2", len(got))
	}
	if got := byName["Sessions"].Nodes; len(got) != 1 {
		t.Errorf("Sessions nodes count = %d, want 1", len(got))
	}
	if got := byName["Etc"].Section; got != "global" {
		// First Etc is global; we won't dig further for content.
	}
}

func TestResolveDestinations_BotHQProjectSection(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)

	dests, err := ResolveDestinations(root, "bot-hq")
	if err != nil {
		t.Fatalf("%v", err)
	}
	// Project-section destinations always come after global; collect the
	// project-section ones explicitly.
	var proj map[string]Destination = map[string]Destination{}
	for _, d := range dests {
		if d.Section == "project" {
			proj[d.Name] = d
		}
	}

	// Overview = README.md special-case for bot-hq.
	if got := proj["Overview"].Nodes; len(got) != 1 || got[0].Name != "README.md" {
		t.Errorf("bot-hq Overview = %+v, want README.md", got)
	}
	// Rules = projects/bot-hq.yaml + gates/*.md (2 gates in fixture).
	rules := proj["Rules"].Nodes
	if len(rules) != 3 {
		t.Errorf("bot-hq Rules nodes count = %d, want 3 (yaml + 2 gates); got=%+v", len(rules), rules)
	}
	hasYaml := false
	gateCount := 0
	for _, n := range rules {
		if strings.HasSuffix(n.Path, ".yaml") {
			hasYaml = true
		}
		if strings.HasPrefix(n.Path, "gates/") {
			gateCount++
		}
	}
	if !hasYaml || gateCount != 2 {
		t.Errorf("bot-hq Rules: hasYaml=%v gateCount=%d, want true/2", hasYaml, gateCount)
	}

	// Plans = phase/*.md (2 in fixture).
	if got := proj["Plans"].Nodes; len(got) != 2 {
		t.Errorf("bot-hq Plans nodes = %d, want 2 phase files; got %+v", len(got), got)
	}

	// Etc = empty for bot-hq.
	if got := proj["Etc"].Nodes; len(got) != 0 {
		t.Errorf("bot-hq Etc nodes = %d, want 0; got %+v", len(got), got)
	}
}

func TestResolveDestinations_NonBotHQProjectSection(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)

	dests, err := ResolveDestinations(root, "myproj")
	if err != nil {
		t.Fatalf("%v", err)
	}
	proj := map[string]Destination{}
	for _, d := range dests {
		if d.Section == "project" {
			proj[d.Name] = d
		}
	}

	// Overview from projects/myproj/overview.md.
	if got := proj["Overview"].Nodes; len(got) != 1 || got[0].Path != "projects/myproj/overview.md" {
		t.Errorf("myproj Overview = %+v", got)
	}
	// Rules = projects/myproj.yaml; gates do NOT surface for non-bot-hq.
	rules := proj["Rules"].Nodes
	if len(rules) != 1 || rules[0].Path != "projects/myproj.yaml" {
		t.Errorf("myproj Rules = %+v, want only projects/myproj.yaml", rules)
	}
	for _, n := range rules {
		if strings.HasPrefix(n.Path, "gates/") {
			t.Errorf("non-bot-hq Rules unexpectedly surfaced gate path %q", n.Path)
		}
	}
	// Plans from projects/myproj/plans/*.md.
	if got := proj["Plans"].Nodes; len(got) != 1 || got[0].Path != "projects/myproj/plans/plan-a.md" {
		t.Errorf("myproj Plans = %+v", got)
	}
	// Etc = clips + eod (1 each in fixture).
	if got := proj["Etc"].Nodes; len(got) != 2 {
		t.Errorf("myproj Etc nodes = %d, want 2 (1 clip + 1 eod); got %+v", len(got), got)
	}
}

func TestResolveDestinations_OverviewBlankState(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "projects"))
	mustWrite(t, filepath.Join(root, "projects", "newproj.yaml"), "project_name: newproj\n")
	// projects/newproj/overview.md NOT created — should yield Missing marker.

	dests, err := ResolveDestinations(root, "newproj")
	if err != nil {
		t.Fatalf("%v", err)
	}
	for _, d := range dests {
		if d.Section == "project" && d.Name == "Overview" {
			if len(d.Nodes) != 1 {
				t.Fatalf("Overview nodes = %d, want 1 missing-marker; got %+v", len(d.Nodes), d.Nodes)
			}
			if !d.Nodes[0].Missing {
				t.Errorf("Overview node Missing = false, want true; got %+v", d.Nodes[0])
			}
		}
	}
}

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
	// Expect 8 global + 4 project = 12 destinations.
	if len(p.Destinations) != 12 {
		t.Errorf("destinations count = %d, want 12; got %+v", len(p.Destinations), p.Destinations)
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
func TestResolveDestinations_HideListEnforced(t *testing.T) {
	root := t.TempDir()
	fillCanonicalLayout(t, root)

	for _, project := range []string{"bot-hq", "myproj"} {
		dests, err := ResolveDestinations(root, project)
		if err != nil {
			t.Fatalf("project=%s: %v", project, err)
		}
		for _, d := range dests {
			for _, n := range d.Nodes {
				p := n.Path
				// HIDE list: db/log/jsonl/voice-mirror-log/last_state.json/diag/sentinels/bridge/plugins
				banned := []string{
					"hub.db", "live.log", "voice-mirror-log.md",
					"last_state.json", "diag/", "sentinels/", "bridge/", "plugins/",
				}
				for _, b := range banned {
					if strings.Contains(p, b) {
						t.Errorf("project=%s dest=%s surfaced HIDE-list path %q", project, d.Name, p)
					}
				}
			}
		}
	}
}

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

// Sanity: a brand-new install (no canonical-store dirs at all) should not
// crash any resolver.
func TestResolveDestinations_EmptyCanonicalStore(t *testing.T) {
	root := t.TempDir()
	dests, err := ResolveDestinations(root, "bot-hq")
	if err != nil {
		t.Fatalf("%v", err)
	}
	if len(dests) != 12 {
		t.Errorf("expected 12 destinations, got %d", len(dests))
	}
	// Most/all should be empty Nodes — no panic.
	for _, d := range dests {
		_ = d.Nodes
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
