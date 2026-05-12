package webui

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fixtureProject lays out a minimal canonical-store project for tree-walk
// testing: projects/<p>.yaml + the 9 canonical subdirs each with a README.
// Returns the relative root path the caller should pass to BuildFilteredTree.
func fixtureProject(t *testing.T, canonRoot, project, yamlBody string) string {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(canonRoot, "projects", project), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(canonRoot, "projects", project+".yaml"), []byte(yamlBody), 0o644); err != nil {
		t.Fatal(err)
	}
	for _, sub := range []string{"architecture", "audit-notes", "clips", "conventions", "decisions", "eod", "glossary", "plans", "tasks"} {
		dir := filepath.Join(canonRoot, "projects", project, sub)
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "README.md"), []byte("# "+sub+"\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(canonRoot, "projects", project, "README.md"), []byte("# "+project+"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(canonRoot, "projects", project, "INDEX.md"), []byte("# index\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	return filepath.ToSlash(filepath.Join("projects", project))
}

func TestBuildFilteredTree_HiddenAtRootSkipped(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "hub.db"), "db")
	mustWrite(t, filepath.Join(root, "live.log"), "log")
	mustWrite(t, filepath.Join(root, "README.md"), "ok")
	if err := os.MkdirAll(filepath.Join(root, "diag"), 0o755); err != nil {
		t.Fatal(err)
	}
	mustWrite(t, filepath.Join(root, "diag", "miss.jsonl"), "{}")

	tree, err := BuildFilteredTree(root, "")
	if err != nil {
		t.Fatal(err)
	}
	names := nodeNames(tree)
	for _, banned := range []string{"hub.db", "live.log", "diag"} {
		if contains(names, banned) {
			t.Errorf("hidden entry %q surfaced in tree: %v", banned, names)
		}
	}
	if !contains(names, "README.md") {
		t.Errorf("expected README.md in tree, got %v", names)
	}
}

func TestBuildFilteredTree_ExtensionsAllowlist_Classification(t *testing.T) {
	root := t.TempDir()
	yaml := `project_name: "bot-hq"
remote_url: ""
extensions:
  brain_duo_operational:
    - phase
    - ratchets
  foundational_anchors:
    - vision.md
`
	rootRel := fixtureProject(t, root, "bot-hq", yaml)
	// Add extension dirs/files actually on disk so the tree walks them.
	if err := os.MkdirAll(filepath.Join(root, "projects", "bot-hq", "phase"), 0o755); err != nil {
		t.Fatal(err)
	}
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "phase", "phase-z.md"), "")
	if err := os.MkdirAll(filepath.Join(root, "projects", "bot-hq", "ratchets"), 0o755); err != nil {
		t.Fatal(err)
	}
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "ratchets", "active.md"), "")
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "vision.md"), "")

	tree, err := BuildFilteredTree(root, rootRel)
	if err != nil {
		t.Fatal(err)
	}
	byName := map[string]TreeNode{}
	for _, n := range tree {
		byName[n.Name] = n
	}
	if got := byName["phase"].Class; got != "brain_duo_operational" {
		t.Errorf("phase class = %q, want brain_duo_operational", got)
	}
	if got := byName["ratchets"].Class; got != "brain_duo_operational" {
		t.Errorf("ratchets class = %q, want brain_duo_operational", got)
	}
	if got := byName["vision.md"].Class; got != "foundational_anchors" {
		t.Errorf("vision.md class = %q, want foundational_anchors", got)
	}
	if got := byName["architecture"].Class; got != "architecture" {
		t.Errorf("architecture class = %q, want architecture (canonical subdir)", got)
	}
	if got := byName["README.md"].Class; got != "overview" {
		t.Errorf("README.md class = %q, want overview", got)
	}
}

func TestBuildFilteredTree_CatchAll_ProjectPrivate(t *testing.T) {
	root := t.TempDir()
	rootRel := fixtureProject(t, root, "myproj", `project_name: myproj
remote_url: ""
`)
	mustWrite(t, filepath.Join(root, "projects", "myproj", "scratch.md"), "")

	tree, err := BuildFilteredTree(root, rootRel)
	if err != nil {
		t.Fatal(err)
	}
	var got TreeNode
	for _, n := range tree {
		if n.Name == "scratch.md" {
			got = n
			break
		}
	}
	if got.Class != "project_private" {
		t.Errorf("scratch.md class = %q, want project_private (PB-Q1 catch-all)", got.Class)
	}
}

func TestBuildFilteredTree_VoiceMirrorLog_AllowedInsideProject(t *testing.T) {
	root := t.TempDir()
	rootRel := fixtureProject(t, root, "bot-hq", `project_name: "bot-hq"`)
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "voice-mirror-log.md"), "")

	tree, err := BuildFilteredTree(root, rootRel)
	if err != nil {
		t.Fatal(err)
	}
	if !contains(nodeNames(tree), "voice-mirror-log.md") {
		t.Errorf("voice-mirror-log.md should surface inside projects/<p>/ (Rain's BRAIN-2nd context-aware note): %v", nodeNames(tree))
	}
}

func TestBuildFilteredTree_VoiceMirrorLog_HiddenAtRoot(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "voice-mirror-log.md"), "")

	tree, err := BuildFilteredTree(root, "")
	if err != nil {
		t.Fatal(err)
	}
	if contains(nodeNames(tree), "voice-mirror-log.md") {
		t.Error("voice-mirror-log.md should be hidden at CL root per plan §2.1")
	}
}

func TestBuildFilteredTree_AgentDir_AllowlistedOnly(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "brian"), 0o755); err != nil {
		t.Fatal(err)
	}
	mustWrite(t, filepath.Join(root, "brian", "last_state.json"), "{}")
	mustWrite(t, filepath.Join(root, "brian", "discipline-anchors.md"), "anchors")

	tree, err := BuildFilteredTree(root, "")
	if err != nil {
		t.Fatal(err)
	}
	var brianNode TreeNode
	for _, n := range tree {
		if n.Name == "brian" {
			brianNode = n
			break
		}
	}
	if brianNode.Name == "" {
		t.Fatalf("brian/ dir missing from tree: %v", nodeNames(tree))
	}
	childNames := nodeNames(brianNode.Children)
	if !contains(childNames, "discipline-anchors.md") {
		t.Errorf("discipline-anchors.md should surface under brian/: %v", childNames)
	}
	if contains(childNames, "last_state.json") {
		t.Errorf("last_state.json should be hidden under agent dir: %v", childNames)
	}
}

func TestBuildFilteredTree_ChildrenInheritClassFromSubdir(t *testing.T) {
	root := t.TempDir()
	rootRel := fixtureProject(t, root, "bot-hq", `project_name: "bot-hq"`)
	// Add a nested file inside architecture/ — should inherit "architecture" class.
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "architecture", "sessions.md"), "")

	tree, err := BuildFilteredTree(root, rootRel)
	if err != nil {
		t.Fatal(err)
	}
	// Find architecture dir node and verify children carry class.
	var archNode TreeNode
	for _, n := range tree {
		if n.Name == "architecture" {
			archNode = n
			break
		}
	}
	for _, child := range archNode.Children {
		if child.Class != "architecture" {
			t.Errorf("nested architecture file %s class = %q, want architecture", child.Name, child.Class)
		}
	}
}

func TestBuildFilteredTree_RequiresCanonRoot(t *testing.T) {
	_, err := BuildFilteredTree("", "projects/bot-hq")
	if err == nil {
		t.Error("expected error when canonRoot empty")
	}
	if !strings.Contains(err.Error(), "canonRoot required") {
		t.Errorf("unexpected error: %v", err)
	}
}

func nodeNames(nodes []TreeNode) []string {
	out := make([]string, 0, len(nodes))
	for _, n := range nodes {
		out = append(out, n.Name)
	}
	return out
}

func contains(haystack []string, needle string) bool {
	for _, s := range haystack {
		if s == needle {
			return true
		}
	}
	return false
}
