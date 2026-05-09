// Package vault implements file-based secret-storage at ~/.bot-hq/secrets/
// per phase-t.md v5 T-8.10 + R52 reference-pointer pattern.
//
// MVP scope: unencrypted-but-permissioned. Each secret = one file at
// <root>/<name> with 0600 permissions; vault root dir 0700. Filesystem
// perms enforce isolation; production-class encryption-at-rest (age /
// sops integration) is Phase V scope.
//
// Migration path: GetWithEnvFallback wraps Get with os.Getenv fallback
// to support graceful migration from existing env-var configs (e.g.,
// DEEPSEEK_API_KEY) without breaking running agents during cutover.
//
// Layering: vault imports stdlib only (no internal deps) — leaf package
// per circular-import-avoidance precedent (T-8.5a/T-8.6/T-8.9a).

package vault

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// FileVault stores secrets as files under root with 0600 perms.
type FileVault struct {
	root string
}

// New constructs a FileVault rooted at the given path. Creates the
// directory tree with 0700 perms if missing. Returns error on
// permission-mode mismatch when dir already exists with looser perms.
func New(root string) (*FileVault, error) {
	if root == "" {
		return nil, errors.New("vault root is required")
	}
	if err := os.MkdirAll(root, 0o700); err != nil {
		return nil, fmt.Errorf("mkdir vault root: %w", err)
	}
	info, err := os.Stat(root)
	if err != nil {
		return nil, fmt.Errorf("stat vault root: %w", err)
	}
	if info.Mode().Perm() != 0o700 {
		// Tighten perms to 0700 if user/admin loosened them
		if err := os.Chmod(root, 0o700); err != nil {
			return nil, fmt.Errorf("chmod vault root to 0700: %w", err)
		}
	}
	return &FileVault{root: root}, nil
}

// ErrSecretNotFound indicates the requested secret name has no value
// in the vault (file does not exist).
var ErrSecretNotFound = errors.New("secret not found")

// GetSecret returns the secret value for name. Returns ErrSecretNotFound
// when the secret does not exist.
func (v *FileVault) GetSecret(name string) (string, error) {
	if err := validName(name); err != nil {
		return "", err
	}
	data, err := os.ReadFile(filepath.Join(v.root, name))
	if err != nil {
		if os.IsNotExist(err) {
			return "", fmt.Errorf("%w: %q", ErrSecretNotFound, name)
		}
		return "", fmt.Errorf("read secret %q: %w", name, err)
	}
	return strings.TrimRight(string(data), "\n"), nil
}

// SetSecret writes value for name with 0600 perms via atomic-rename.
func (v *FileVault) SetSecret(name, value string) error {
	if err := validName(name); err != nil {
		return err
	}
	target := filepath.Join(v.root, name)
	tmp := target + ".tmp"
	if err := os.WriteFile(tmp, []byte(value), 0o600); err != nil {
		return fmt.Errorf("write tmp: %w", err)
	}
	if err := os.Rename(tmp, target); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("atomic rename: %w", err)
	}
	return nil
}

// DeleteSecret removes name from the vault. Idempotent — no error when
// secret does not exist.
func (v *FileVault) DeleteSecret(name string) error {
	if err := validName(name); err != nil {
		return err
	}
	if err := os.Remove(filepath.Join(v.root, name)); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("remove secret %q: %w", name, err)
	}
	return nil
}

// GetWithEnvFallback returns the secret for name if present in vault;
// otherwise falls back to os.Getenv(envVarName). Returns
// ErrSecretNotFound when neither vault nor env-var has a value.
//
// Migration helper: caller-side agentconfig.BuildSpawnEnv can be wired
// to use this for back-compat lookup of existing env-var configs
// during vault-cutover (T-8.10b followup).
func (v *FileVault) GetWithEnvFallback(name, envVarName string) (string, error) {
	if val, err := v.GetSecret(name); err == nil {
		return val, nil
	} else if !errors.Is(err, ErrSecretNotFound) {
		return "", err
	}
	if envVarName == "" {
		return "", fmt.Errorf("%w: %q (no env-var fallback)", ErrSecretNotFound, name)
	}
	if val := os.Getenv(envVarName); val != "" {
		return val, nil
	}
	return "", fmt.Errorf("%w: vault[%q] missing AND env[%s] empty", ErrSecretNotFound, name, envVarName)
}

// ListSecretNames returns all secret names sorted lexically. Skips files
// with ".tmp" suffix (in-flight atomic-write artifacts).
func (v *FileVault) ListSecretNames() ([]string, error) {
	entries, err := os.ReadDir(v.root)
	if err != nil {
		return nil, fmt.Errorf("readdir: %w", err)
	}
	names := make([]string, 0, len(entries))
	for _, e := range entries {
		if e.IsDir() || strings.HasSuffix(e.Name(), ".tmp") {
			continue
		}
		names = append(names, e.Name())
	}
	sort.Strings(names)
	return names, nil
}

// Root returns the vault root path (for diagnostics + tests).
func (v *FileVault) Root() string { return v.root }

// validName rejects empty names + names containing path separators
// (defends against path-traversal: e.g., "../etc/passwd").
func validName(name string) error {
	if name == "" {
		return errors.New("secret name is required")
	}
	if strings.ContainsAny(name, "/\\") || name == "." || name == ".." {
		return fmt.Errorf("invalid secret name %q (no path separators)", name)
	}
	return nil
}
