package vault

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func newTestVault(t *testing.T) *FileVault {
	t.Helper()
	v, err := New(filepath.Join(t.TempDir(), "secrets"))
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	return v
}

func TestNew_validation(t *testing.T) {
	if _, err := New(""); err == nil {
		t.Error("expected error for empty root")
	}
}

func TestNew_dirPermissionsTightened(t *testing.T) {
	root := filepath.Join(t.TempDir(), "secrets")
	// Pre-create with looser perms
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatalf("pre-mkdir: %v", err)
	}
	v, err := New(root)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	info, _ := os.Stat(v.Root())
	if info.Mode().Perm() != 0o700 {
		t.Errorf("dir perms = %o, want 0700 (tightened)", info.Mode().Perm())
	}
}

func TestSetGet_roundTrip(t *testing.T) {
	v := newTestVault(t)
	if err := v.SetSecret("api-key", "secret-value-12345"); err != nil {
		t.Fatalf("SetSecret: %v", err)
	}
	got, err := v.GetSecret("api-key")
	if err != nil {
		t.Fatalf("GetSecret: %v", err)
	}
	if got != "secret-value-12345" {
		t.Errorf("GetSecret = %q, want secret-value-12345", got)
	}
}

func TestSet_filePermsAre0600(t *testing.T) {
	v := newTestVault(t)
	_ = v.SetSecret("k", "v")
	info, err := os.Stat(filepath.Join(v.Root(), "k"))
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Errorf("file perms = %o, want 0600", info.Mode().Perm())
	}
}

func TestGet_nonexistentReturnsErrSecretNotFound(t *testing.T) {
	v := newTestVault(t)
	_, err := v.GetSecret("missing")
	if err == nil {
		t.Fatal("expected error for missing secret")
	}
	if !errors.Is(err, ErrSecretNotFound) {
		t.Errorf("err = %v, want errors.Is ErrSecretNotFound", err)
	}
}

func TestDelete_idempotentOnMissing(t *testing.T) {
	v := newTestVault(t)
	if err := v.DeleteSecret("never-existed"); err != nil {
		t.Errorf("DeleteSecret on missing should be idempotent, got: %v", err)
	}
}

func TestDelete_removesFile(t *testing.T) {
	v := newTestVault(t)
	_ = v.SetSecret("k", "v")
	if err := v.DeleteSecret("k"); err != nil {
		t.Fatalf("DeleteSecret: %v", err)
	}
	if _, err := v.GetSecret("k"); !errors.Is(err, ErrSecretNotFound) {
		t.Errorf("GetSecret after Delete = %v, want ErrSecretNotFound", err)
	}
}

func TestGetWithEnvFallback_vaultPrecedence(t *testing.T) {
	v := newTestVault(t)
	t.Setenv("MY_KEY", "from-env")
	_ = v.SetSecret("my-key", "from-vault")

	got, err := v.GetWithEnvFallback("my-key", "MY_KEY")
	if err != nil {
		t.Fatalf("GetWithEnvFallback: %v", err)
	}
	if got != "from-vault" {
		t.Errorf("got = %q, want 'from-vault' (vault precedence over env)", got)
	}
}

func TestGetWithEnvFallback_envFallbackWhenVaultMissing(t *testing.T) {
	v := newTestVault(t)
	t.Setenv("FALLBACK_KEY", "fallback-value")
	got, err := v.GetWithEnvFallback("not-in-vault", "FALLBACK_KEY")
	if err != nil {
		t.Fatalf("GetWithEnvFallback: %v", err)
	}
	if got != "fallback-value" {
		t.Errorf("got = %q, want fallback-value", got)
	}
}

func TestGetWithEnvFallback_bothMissingReturnsNotFound(t *testing.T) {
	v := newTestVault(t)
	_, err := v.GetWithEnvFallback("missing", "ALSO_MISSING_ENV")
	if !errors.Is(err, ErrSecretNotFound) {
		t.Errorf("err = %v, want ErrSecretNotFound when neither vault nor env has value", err)
	}
}

func TestListSecretNames_returnsSortedSkipsTmp(t *testing.T) {
	v := newTestVault(t)
	_ = v.SetSecret("zeta", "z")
	_ = v.SetSecret("alpha", "a")
	_ = v.SetSecret("middle", "m")
	// Drop a .tmp file directly to simulate in-flight atomic-write
	_ = os.WriteFile(filepath.Join(v.Root(), "stale.tmp"), []byte("x"), 0o600)

	names, err := v.ListSecretNames()
	if err != nil {
		t.Fatalf("ListSecretNames: %v", err)
	}
	if len(names) != 3 {
		t.Errorf("len = %d, want 3 (.tmp skipped)", len(names))
	}
	if names[0] != "alpha" || names[2] != "zeta" {
		t.Errorf("names = %v, want sorted [alpha, middle, zeta]", names)
	}
}

func TestSecretNameValidation_rejectsPathSeparators(t *testing.T) {
	v := newTestVault(t)
	cases := []string{"../escape", "sub/dir", `back\slash`, ".", ""}
	for _, name := range cases {
		t.Run(name, func(t *testing.T) {
			if err := v.SetSecret(name, "x"); err == nil {
				t.Errorf("SetSecret(%q) should reject path-traversal", name)
			}
		})
	}
}
