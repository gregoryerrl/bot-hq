package bootstrap

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestWriteRead_roundTrip(t *testing.T) {
	root := t.TempDir()
	in := Bootstrap{
		Frontmatter: Frontmatter{
			LastSessionID:      "bs26kkej3",
			LastSessionCloseAt: time.Date(2026, 5, 6, 22, 14, 33, 0, time.UTC),
			PhaseOrMilestone:   "phase-n-v3.x-2",
			KeyState:           "Brian on bot-hq main",
			WriteTrigger:       "graceful",
			LastNPeerCoord:     []string{"msg-9821", "msg-9834"},
		},
		Body: "# Free-form summary\n\nDetails here.\n",
	}
	if err := Write(root, "bot-hq", in); err != nil {
		t.Fatal(err)
	}
	got, err := Read(root, "bot-hq")
	if err != nil {
		t.Fatal(err)
	}
	if got == nil {
		t.Fatal("expected bootstrap, got nil")
	}
	if got.Frontmatter.LastSessionID != "bs26kkej3" {
		t.Errorf("session id: got %q", got.Frontmatter.LastSessionID)
	}
	if got.Frontmatter.PhaseOrMilestone != "phase-n-v3.x-2" {
		t.Errorf("phase: got %q", got.Frontmatter.PhaseOrMilestone)
	}
	if !strings.Contains(got.Body, "Free-form summary") {
		t.Errorf("body lost: %q", got.Body)
	}
}

func TestRead_missing_returnsNilNil(t *testing.T) {
	root := t.TempDir()
	got, err := Read(root, "nonexistent")
	if err != nil {
		t.Fatalf("missing should not error: %v", err)
	}
	if got != nil {
		t.Errorf("missing should return nil, got %+v", got)
	}
}

func TestRead_staleFlagsWriteTrigger(t *testing.T) {
	root := t.TempDir()
	in := Bootstrap{
		Frontmatter: Frontmatter{
			LastSessionCloseAt: time.Now().Add(-48 * time.Hour),
			WriteTrigger:       "graceful",
		},
		Body: "stale",
	}
	if err := Write(root, "p1", in); err != nil {
		t.Fatal(err)
	}
	got, err := Read(root, "p1")
	if err != nil {
		t.Fatal(err)
	}
	if got.Frontmatter.WriteTrigger != "stale" {
		t.Errorf("expected stale flag; got %q", got.Frontmatter.WriteTrigger)
	}
}

func TestWrite_atomicNoTornFile(t *testing.T) {
	root := t.TempDir()
	in := Bootstrap{Body: "test"}
	if err := Write(root, "p1", in); err != nil {
		t.Fatal(err)
	}
	// .tmp file must not exist post-write.
	tmp := filepath.Join(root, "projects", "p1", "bootstrap.md.tmp")
	if _, err := os.Stat(tmp); !os.IsNotExist(err) {
		t.Errorf("tmp file should be removed after rename: %v", err)
	}
}

func TestDecode_noFrontmatter_treatsAsBody(t *testing.T) {
	raw := "Just a body, no frontmatter.\n"
	got, err := Decode(raw)
	if err != nil {
		t.Fatal(err)
	}
	if got.Body != raw {
		t.Errorf("body should be whole input: got %q", got.Body)
	}
	if got.Frontmatter.LastSessionID != "" {
		t.Errorf("no frontmatter expected: got %+v", got.Frontmatter)
	}
}

func TestDecode_malformedFrontmatter_treatsAsBody(t *testing.T) {
	raw := "---\nlast_session_id: foo\n(no closing delimiter)\n"
	got, err := Decode(raw)
	if err != nil {
		t.Fatal(err)
	}
	if got.Body != raw {
		t.Errorf("malformed frontmatter should fall back to body-as-whole: got %q", got.Body)
	}
}
