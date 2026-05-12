// Package cl — seed.go: Phase Z S2 SeedProjectSubdirs helper.
//
// Idempotent seeding of a project's canonical 9-subdir layout plus any
// extension-class directories declared in <p>.yaml. Each subdir gets a
// minimal README.md if absent (existing READMEs are never overwritten).
//
// Callers:
//   - cl-index --fix (load yaml extensions, pass dir-class as varargs)
//   - handleProjectRegister (register-time seed; empty extensionDirs varargs)
//
// Idempotency invariant: running this twice on the same project produces
// no on-disk diff. mkdir errors with IsExist are tolerated; README writes
// skip when the file already exists.

package cl

import (
	"fmt"
	"os"
	"path/filepath"
)

// SeedProjectSubdirs ensures projects/<project>/<subdir>/ + README.md exist
// for every canonical subdir + every declared extension directory.
//
// canonRoot is the CL root (e.g., ~/.bot-hq). extensionDirs is the
// dir-class extensions from <p>.yaml extensions:* blocks (basenames
// without a `.` per the filename convention — files like vision.md are
// NOT seeded; they exist or don't).
//
// Returns first error encountered; subsequent subdirs are skipped on
// error (caller may re-run after fixing the underlying cause —
// idempotency makes partial-progress safe).
func SeedProjectSubdirs(canonRoot, project string, extensionDirs ...string) error {
	if canonRoot == "" {
		return fmt.Errorf("seed: canonRoot required")
	}
	if project == "" {
		return fmt.Errorf("seed: project required")
	}
	projDir := filepath.Join(canonRoot, "projects", project)
	if err := os.MkdirAll(projDir, 0o755); err != nil {
		return fmt.Errorf("seed: mkdir projDir: %w", err)
	}

	for _, sub := range indexSubdirs {
		if err := seedOneSubdir(projDir, sub, subdirDescription[sub]); err != nil {
			return err
		}
	}

	for _, ext := range extensionDirs {
		if ext == "" {
			continue
		}
		desc := fmt.Sprintf("Extension directory declared in %s.yaml.", project)
		if err := seedOneSubdir(projDir, ext, desc); err != nil {
			return err
		}
	}

	return nil
}

// seedOneSubdir mkdirs the subdir and writes a minimal README.md if
// absent. Existing READMEs are never overwritten.
func seedOneSubdir(projDir, sub, description string) error {
	subPath := filepath.Join(projDir, sub)
	if err := os.MkdirAll(subPath, 0o755); err != nil {
		return fmt.Errorf("seed: mkdir %s: %w", sub, err)
	}
	readmePath := filepath.Join(subPath, "README.md")
	if _, err := os.Stat(readmePath); err == nil {
		return nil
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("seed: stat %s: %w", readmePath, err)
	}
	body := fmt.Sprintf("# %s\n\n%s\n", sub, description)
	if err := os.WriteFile(readmePath, []byte(body), 0o644); err != nil {
		return fmt.Errorf("seed: write %s: %w", readmePath, err)
	}
	return nil
}
