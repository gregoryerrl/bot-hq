package hub

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDefaultConfig(t *testing.T) {
	cfg := DefaultConfig()
	if cfg.Hub.ClivePort != 3847 {
		t.Errorf("expected port 3847, got %d", cfg.Hub.ClivePort)
	}
	if cfg.Clive.Voice != "Iapetus" {
		t.Errorf("expected voice Iapetus, got %s", cfg.Clive.Voice)
	}
}

func TestLoadConfigCreatesDefault(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.toml")
	cfg, err := LoadConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Hub.ClivePort != 3847 {
		t.Errorf("expected default port, got %d", cfg.Hub.ClivePort)
	}
	if _, err := os.Stat(path); os.IsNotExist(err) {
		t.Error("config file should have been created")
	}
}

func TestLoadConfigReadsExisting(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.toml")
	os.WriteFile(path, []byte(`
[hub]
clive_port = 9999

[clive]
voice = "Charon"
`), 0644)

	cfg, err := LoadConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Hub.ClivePort != 9999 {
		t.Errorf("expected port 9999, got %d", cfg.Hub.ClivePort)
	}
	if cfg.Clive.Voice != "Charon" {
		t.Errorf("expected voice Charon, got %s", cfg.Clive.Voice)
	}
}
