package cl

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

// Phase Z S2 SeedProjectSubdirs tests — R39 TEST-ISOLATION via t.TempDir().

func TestSeedProjectSubdirs_CanonicalNineSubdirsCreated(t *testing.T) {
	root := t.TempDir()
	if err := SeedProjectSubdirs(root, "fixture-proj"); err != nil {
		t.Fatalf("seed: %v", err)
	}
	projDir := filepath.Join(root, "projects", "fixture-proj")
	for _, sub := range indexSubdirs {
		subPath := filepath.Join(projDir, sub)
		st, err := os.Stat(subPath)
		if err != nil {
			t.Errorf("missing canonical subdir %s: %v", sub, err)
			continue
		}
		if !st.IsDir() {
			t.Errorf("%s exists but is not a directory", subPath)
		}
		readme := filepath.Join(subPath, "README.md")
		body, err := os.ReadFile(readme)
		if err != nil {
			t.Errorf("missing README %s: %v", readme, err)
			continue
		}
		if !strings.Contains(string(body), "# "+sub) {
			t.Errorf("README for %s missing # %s heading; got %q", sub, sub, string(body))
		}
		if want := subdirDescription[sub]; want != "" && !strings.Contains(string(body), want) {
			t.Errorf("README for %s missing description %q; got %q", sub, want, string(body))
		}
	}
}

func TestSeedProjectSubdirs_ExtensionDirsCreated(t *testing.T) {
	root := t.TempDir()
	if err := SeedProjectSubdirs(root, "fixture-proj", "phase", "ratchets", "env"); err != nil {
		t.Fatalf("seed: %v", err)
	}
	projDir := filepath.Join(root, "projects", "fixture-proj")
	for _, ext := range []string{"phase", "ratchets", "env"} {
		extPath := filepath.Join(projDir, ext)
		if st, err := os.Stat(extPath); err != nil || !st.IsDir() {
			t.Errorf("extension dir %s missing or not a dir: %v", extPath, err)
		}
		readme := filepath.Join(extPath, "README.md")
		body, err := os.ReadFile(readme)
		if err != nil {
			t.Errorf("missing extension README %s: %v", readme, err)
			continue
		}
		if !strings.Contains(string(body), "Extension directory") {
			t.Errorf("extension README missing class hint; got %q", string(body))
		}
	}
}

func TestSeedProjectSubdirs_Idempotent(t *testing.T) {
	root := t.TempDir()
	if err := SeedProjectSubdirs(root, "fixture-proj", "phase"); err != nil {
		t.Fatalf("seed first: %v", err)
	}
	first := snapshotDir(t, filepath.Join(root, "projects", "fixture-proj"))
	if err := SeedProjectSubdirs(root, "fixture-proj", "phase"); err != nil {
		t.Fatalf("seed second: %v", err)
	}
	second := snapshotDir(t, filepath.Join(root, "projects", "fixture-proj"))
	if first != second {
		t.Errorf("idempotency violated:\nfirst:  %s\nsecond: %s", first, second)
	}
}

func TestSeedProjectSubdirs_ExistingReadmeNotOverwritten(t *testing.T) {
	root := t.TempDir()
	projDir := filepath.Join(root, "projects", "fixture-proj", "architecture")
	if err := os.MkdirAll(projDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	custom := "# architecture\n\nUser-authored content, must survive.\n"
	if err := os.WriteFile(filepath.Join(projDir, "README.md"), []byte(custom), 0o644); err != nil {
		t.Fatalf("seed user readme: %v", err)
	}
	if err := SeedProjectSubdirs(root, "fixture-proj"); err != nil {
		t.Fatalf("seed: %v", err)
	}
	got, err := os.ReadFile(filepath.Join(projDir, "README.md"))
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	if string(got) != custom {
		t.Errorf("existing README overwritten; want %q got %q", custom, string(got))
	}
}

func TestSeedProjectSubdirs_EmptyArgsRejected(t *testing.T) {
	cases := []struct {
		name, root, project string
	}{
		{"empty canonRoot", "", "foo"},
		{"empty project", t.TempDir(), ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if err := SeedProjectSubdirs(c.root, c.project); err == nil {
				t.Fatalf("expected error for empty arg")
			}
		})
	}
}

func TestSeedProjectSubdirs_EmptyExtensionDirSkipped(t *testing.T) {
	root := t.TempDir()
	if err := SeedProjectSubdirs(root, "fixture-proj", "", "phase", ""); err != nil {
		t.Fatalf("seed: %v", err)
	}
	projDir := filepath.Join(root, "projects", "fixture-proj")
	if _, err := os.Stat(filepath.Join(projDir, "phase")); err != nil {
		t.Errorf("non-empty ext should still seed: %v", err)
	}
	entries, err := os.ReadDir(projDir)
	if err != nil {
		t.Fatalf("readdir: %v", err)
	}
	for _, e := range entries {
		if e.Name() == "" {
			t.Errorf("empty-string ext leaked as subdir")
		}
	}
}

// snapshotDir returns a deterministic hash of every regular file under
// dir (path + size + content hash). Used to verify idempotency.
func snapshotDir(t *testing.T, dir string) string {
	t.Helper()
	type fileSig struct {
		rel  string
		size int64
		hash string
	}
	var sigs []fileSig
	err := filepath.Walk(dir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.IsDir() {
			return nil
		}
		rel, _ := filepath.Rel(dir, path)
		data, rerr := os.ReadFile(path)
		if rerr != nil {
			return rerr
		}
		sum := sha256.Sum256(data)
		sigs = append(sigs, fileSig{rel: rel, size: info.Size(), hash: hex.EncodeToString(sum[:])})
		return nil
	})
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("walk: %v", err)
	}
	sort.Slice(sigs, func(i, j int) bool { return sigs[i].rel < sigs[j].rel })
	var b strings.Builder
	for _, s := range sigs {
		b.WriteString(s.rel)
		b.WriteString(":")
		b.WriteString(s.hash)
		b.WriteString("\n")
	}
	return b.String()
}
