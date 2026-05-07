package webui

import (
	"os"
	"path/filepath"
	"testing"
)

// TestExecuteVoiceHubTool_ReadFile_ExplicitPath covers Clive reading a
// file when the path is supplied directly.
func TestExecuteVoiceHubTool_ReadFile_ExplicitPath(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "tasks.md"), "# tasks\n")
	s := newTestServerWithProposals(t, root)

	res := s.executeVoiceHubTool("read_file", map[string]interface{}{"path": "tasks.md"})
	if errStr, ok := res["error"].(string); ok && errStr != "" {
		t.Fatalf("unexpected error: %s", errStr)
	}
	if got, _ := res["content"].(string); got != "# tasks\n" {
		t.Errorf("content = %q, want '# tasks\\n'", got)
	}
}

// TestExecuteVoiceHubTool_ReadFile_FocusFallback covers the implicit
// path case — when no path is supplied, Clive reads the file the user
// is currently viewing in the web UI.
func TestExecuteVoiceHubTool_ReadFile_FocusFallback(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "focus.md"), "# focus\n")
	s := newTestServerWithProposals(t, root)
	s.SetWebuiContext(WebuiContext{Project: "bot-hq", CurrentPath: "focus.md", ViewMode: "rendered"})

	res := s.executeVoiceHubTool("read_file", map[string]interface{}{})
	if errStr, ok := res["error"].(string); ok && errStr != "" {
		t.Fatalf("unexpected error: %s", errStr)
	}
	if got, _ := res["content"].(string); got != "# focus\n" {
		t.Errorf("content = %q, want '# focus\\n'", got)
	}
}

// TestExecuteVoiceHubTool_ReadFile_NoPathNoFocus covers the error path
// when neither an explicit path nor a focus context is available.
func TestExecuteVoiceHubTool_ReadFile_NoPathNoFocus(t *testing.T) {
	root := t.TempDir()
	s := newTestServerWithProposals(t, root)

	res := s.executeVoiceHubTool("read_file", map[string]interface{}{})
	if errStr, _ := res["error"].(string); errStr == "" {
		t.Errorf("expected error for no-path-no-focus, got %+v", res)
	}
}

// TestExecuteVoiceHubTool_ProposeFileEdit_StoresProposal covers that a
// propose call populates the in-memory proposal store with the right
// path / content / purpose, returning a proposal_id.
func TestExecuteVoiceHubTool_ProposeFileEdit_StoresProposal(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "projects", "bot-hq"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "plan.md"), "old\n")
	s := newTestServerWithProposals(t, root)

	res := s.executeVoiceHubTool("propose_file_edit", map[string]interface{}{
		"path":    "projects/bot-hq/plan.md",
		"content": "new content\n",
		"purpose": "test purpose",
	})
	if errStr, ok := res["error"].(string); ok && errStr != "" {
		t.Fatalf("unexpected error: %s", errStr)
	}
	id, _ := res["proposal_id"].(string)
	if id == "" {
		t.Fatalf("proposal_id missing: %+v", res)
	}
	p := s.proposals.get(id)
	if p == nil {
		t.Fatalf("proposal %s not in store", id)
	}
	if p.relPath != "projects/bot-hq/plan.md" {
		t.Errorf("relPath = %q", p.relPath)
	}
	if p.content != "new content\n" {
		t.Errorf("content = %q", p.content)
	}
	if p.purpose != "test purpose" {
		t.Errorf("purpose = %q", p.purpose)
	}

	// Disk content should remain unchanged — propose does NOT apply.
	got, _ := os.ReadFile(filepath.Join(root, "projects", "bot-hq", "plan.md"))
	if string(got) != "old\n" {
		t.Errorf("disk content unexpectedly mutated to %q", got)
	}
}

// TestExecuteVoiceHubTool_ProposeFileEdit_FocusFallback covers the
// implicit-path case for proposals.
func TestExecuteVoiceHubTool_ProposeFileEdit_FocusFallback(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "tasks.md"), "old\n")
	s := newTestServerWithProposals(t, root)
	s.SetWebuiContext(WebuiContext{Project: "bot-hq", CurrentPath: "tasks.md", ViewMode: "raw"})

	res := s.executeVoiceHubTool("propose_file_edit", map[string]interface{}{
		"content": "fresh\n",
		"purpose": "voice-driven add",
	})
	if errStr, ok := res["error"].(string); ok && errStr != "" {
		t.Fatalf("unexpected error: %s", errStr)
	}
	if path, _ := res["path"].(string); path != "tasks.md" {
		t.Errorf("path = %q, want tasks.md (from focus)", path)
	}
}

// TestExecuteVoiceHubTool_ProposeFileEdit_RequiresContentAndPurpose
// covers that empty content or purpose returns an error and does not
// register a proposal.
func TestExecuteVoiceHubTool_ProposeFileEdit_RequiresContentAndPurpose(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "tasks.md"), "x\n")
	s := newTestServerWithProposals(t, root)

	for _, args := range []map[string]interface{}{
		{"path": "tasks.md", "purpose": "p"},                    // no content
		{"path": "tasks.md", "content": "c"},                    // no purpose
		{"content": "c", "purpose": "p"},                        // no path, no focus
		{"path": "../escape.md", "content": "c", "purpose": "p"}, // path-traversal rejected
	} {
		res := s.executeVoiceHubTool("propose_file_edit", args)
		if errStr, _ := res["error"].(string); errStr == "" {
			t.Errorf("expected error for args %+v, got %+v", args, res)
		}
	}
}

// TestHubToolDeclarations_IncludesEditTools is a thin guard that the
// new read_file + propose_file_edit declarations remain present so the
// Gemini Live setup-msg surfaces them.
func TestHubToolDeclarations_IncludesEditTools(t *testing.T) {
	decls := hubToolDeclarations()
	got := map[string]bool{}
	for _, d := range decls {
		if name, ok := d["name"].(string); ok {
			got[name] = true
		}
	}
	for _, want := range []string{"read_file", "propose_file_edit"} {
		if !got[want] {
			t.Errorf("hubToolDeclarations missing %q", want)
		}
	}
}
