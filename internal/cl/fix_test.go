package cl

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Phase Z S2 FixProject tests — R39 TEST-ISOLATION via t.TempDir().

func TestEnsureRemoteURLPresent_AlreadyPresent_NoChange(t *testing.T) {
	src := []byte(`remote_url: "git@github.com:org/foo.git"
project_name: "foo"
`)
	out, changed := ensureRemoteURLPresent(src)
	if changed {
		t.Errorf("changed=true on already-present remote_url")
	}
	if string(out) != string(src) {
		t.Errorf("bytes mutated despite changed=false: %q", string(out))
	}
}

func TestEnsureRemoteURLPresent_Absent_InjectsBeforeProjectName(t *testing.T) {
	src := []byte(`# header comment
project_name: "foo"
other: value
`)
	out, changed := ensureRemoteURLPresent(src)
	if !changed {
		t.Fatalf("changed=false on absent remote_url")
	}
	gotStr := string(out)
	if !strings.Contains(gotStr, "remote_url: \"\"\n") {
		t.Errorf("inject missing; got %q", gotStr)
	}
	rIdx := strings.Index(gotStr, "remote_url:")
	pIdx := strings.Index(gotStr, "project_name:")
	if rIdx < 0 || pIdx < 0 || rIdx > pIdx {
		t.Errorf("expected remote_url before project_name; got rIdx=%d pIdx=%d in %q", rIdx, pIdx, gotStr)
	}
	if !strings.Contains(gotStr, "# header comment\n") {
		t.Errorf("comment lost during surgical append: %q", gotStr)
	}
}

func TestEnsureRemoteURLPresent_NoProjectName_NoOp(t *testing.T) {
	src := []byte("just_some_other_key: value\n")
	out, changed := ensureRemoteURLPresent(src)
	if changed {
		t.Errorf("changed=true without project_name anchor")
	}
	if string(out) != string(src) {
		t.Errorf("bytes mutated despite changed=false")
	}
}

func TestEnsureRemoteURLPresent_CommentedRemoteURL_StillInjects(t *testing.T) {
	src := []byte(`# remote_url: "old.example.com/foo"
project_name: "foo"
`)
	_, changed := ensureRemoteURLPresent(src)
	if !changed {
		t.Errorf("commented remote_url should NOT count as declared; expected injection")
	}
}

func TestFixProject_SeedsCanonicalNineDirs(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "foo", `project_name: "foo"
remote_url: ""
`)
	if err := c.FixProject("foo"); err != nil {
		t.Fatalf("fix: %v", err)
	}
	projDir := filepath.Join(c.root, "projects", "foo")
	for _, sub := range indexSubdirs {
		if _, err := os.Stat(filepath.Join(projDir, sub, "README.md")); err != nil {
			t.Errorf("missing canonical README for %s: %v", sub, err)
		}
	}
}

func TestFixProject_SeedsExtensionDirsOnly(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "bot-hq", `project_name: "bot-hq"
remote_url: ""
extensions:
  brain_duo_operational:
    - phase
    - ratchets
    - discipline-log.md
  foundational_anchors:
    - vision.md
`)
	if err := c.FixProject("bot-hq"); err != nil {
		t.Fatalf("fix: %v", err)
	}
	projDir := filepath.Join(c.root, "projects", "bot-hq")

	if _, err := os.Stat(filepath.Join(projDir, "phase", "README.md")); err != nil {
		t.Errorf("dir-class extension 'phase' not seeded: %v", err)
	}
	if _, err := os.Stat(filepath.Join(projDir, "ratchets", "README.md")); err != nil {
		t.Errorf("dir-class extension 'ratchets' not seeded: %v", err)
	}

	if _, err := os.Stat(filepath.Join(projDir, "discipline-log.md")); err == nil {
		t.Errorf("file-class extension 'discipline-log.md' incorrectly seeded as dir")
	}
	if _, err := os.Stat(filepath.Join(projDir, "vision.md")); err == nil {
		t.Errorf("file-class extension 'vision.md' incorrectly seeded as dir")
	}
}

func TestFixProject_SurgicalRemoteURLAppend(t *testing.T) {
	c := newCLFixture(t)
	original := `# header
project_name: "needs-remote-url"
library:
  schema:
    - "README.md"
`
	writeYaml(t, c, "needs-remote-url", original)
	if err := c.FixProject("needs-remote-url"); err != nil {
		t.Fatalf("fix: %v", err)
	}
	got, err := os.ReadFile(filepath.Join(c.root, "projects", "needs-remote-url.yaml"))
	if err != nil {
		t.Fatalf("read yaml: %v", err)
	}
	gotStr := string(got)
	if !strings.Contains(gotStr, "remote_url: \"\"") {
		t.Errorf("remote_url not injected; got %q", gotStr)
	}
	if !strings.Contains(gotStr, "# header") {
		t.Errorf("comment lost; got %q", gotStr)
	}
	if !strings.Contains(gotStr, "library:\n  schema:\n    - \"README.md\"\n") {
		t.Errorf("downstream content reformatted; got %q", gotStr)
	}
}

func TestFixProject_Idempotent(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "foo", `project_name: "foo"
`)
	if err := c.FixProject("foo"); err != nil {
		t.Fatalf("fix first: %v", err)
	}
	yaml1, _ := os.ReadFile(filepath.Join(c.root, "projects", "foo.yaml"))
	snap1 := snapshotDir(t, filepath.Join(c.root, "projects", "foo"))

	if err := c.FixProject("foo"); err != nil {
		t.Fatalf("fix second: %v", err)
	}
	yaml2, _ := os.ReadFile(filepath.Join(c.root, "projects", "foo.yaml"))
	snap2 := snapshotDir(t, filepath.Join(c.root, "projects", "foo"))

	if string(yaml1) != string(yaml2) {
		t.Errorf("yaml mutated on second run:\nfirst:  %q\nsecond: %q", string(yaml1), string(yaml2))
	}
	if snap1 != snap2 {
		t.Errorf("dir snapshot drift on second run")
	}
}

func TestFixProject_MissingYaml_ErrNotFound(t *testing.T) {
	c := newCLFixture(t)
	err := c.FixProject("does-not-exist")
	if err == nil {
		t.Fatalf("expected error for missing yaml")
	}
}

func TestFixAll_AppliesToAllProjects(t *testing.T) {
	c := newCLFixture(t)
	writeYaml(t, c, "alpha", "project_name: \"alpha\"\n")
	writeYaml(t, c, "beta", "project_name: \"beta\"\n")
	fixed, err := c.FixAll()
	if err != nil {
		t.Fatalf("fix-all: %v", err)
	}
	if len(fixed) != 2 {
		t.Errorf("want 2 fixed projects, got %d (%v)", len(fixed), fixed)
	}
	for _, p := range []string{"alpha", "beta"} {
		if _, err := os.Stat(filepath.Join(c.root, "projects", p, "architecture", "README.md")); err != nil {
			t.Errorf("FixAll missed %s: %v", p, err)
		}
	}
}

func TestDirClassExtensions_FiltersByDotPresence(t *testing.T) {
	got := dirClassExtensions([]string{"phase", "vision.md", "ratchets", "bootstrap.md", "", "env"})
	want := []string{"phase", "ratchets", "env"}
	if len(got) != len(want) {
		t.Fatalf("len mismatch: got %v, want %v", got, want)
	}
	for i, g := range got {
		if g != want[i] {
			t.Errorf("idx %d: got %q want %q", i, g, want[i])
		}
	}
}
