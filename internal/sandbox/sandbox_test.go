package sandbox

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// ====== InProcessSandbox ======

func TestInProcessSandbox_spawnExecTeardown(t *testing.T) {
	s := NewInProcessSandbox(0)
	sid, err := s.Spawn()
	if err != nil {
		t.Fatalf("Spawn: %v", err)
	}
	if sid == "" {
		t.Error("session id empty")
	}

	stdout, stderr, code, err := s.Exec(sid, "echo hello-from-sandbox")
	if err != nil {
		t.Fatalf("Exec: %v (stderr=%s)", err, stderr)
	}
	if !strings.Contains(stdout, "hello-from-sandbox") {
		t.Errorf("stdout = %q, want hello-from-sandbox", stdout)
	}
	if code != 0 {
		t.Errorf("exit-code = %d, want 0", code)
	}

	if err := s.Teardown(sid); err != nil {
		t.Fatalf("Teardown: %v", err)
	}
	// Post-teardown: session is gone; Exec should error.
	if _, _, _, err := s.Exec(sid, "echo after-teardown"); err == nil {
		t.Error("expected error executing in torn-down session")
	}
}

func TestInProcessSandbox_unknownSessionExec(t *testing.T) {
	s := NewInProcessSandbox(0)
	if _, _, _, err := s.Exec("nonexistent-session-id", "echo x"); err == nil {
		t.Error("expected error for unknown session")
	}
}

func TestInProcessSandbox_teardownIdempotent(t *testing.T) {
	s := NewInProcessSandbox(0)
	if err := s.Teardown("never-spawned-id"); err != nil {
		t.Errorf("teardown unknown should be idempotent (nil err), got: %v", err)
	}
}

func TestInProcessSandbox_execNonZeroExitCaptured(t *testing.T) {
	s := NewInProcessSandbox(0)
	sid, _ := s.Spawn()
	defer s.Teardown(sid)

	_, _, code, err := s.Exec(sid, "exit 42")
	if err != nil {
		t.Fatalf("Exec error: %v", err)
	}
	if code != 42 {
		t.Errorf("exit-code = %d, want 42 (non-zero captured)", code)
	}
}

func TestInProcessSandbox_stdoutStderrSeparate(t *testing.T) {
	s := NewInProcessSandbox(0)
	sid, _ := s.Spawn()
	defer s.Teardown(sid)

	stdout, stderr, code, err := s.Exec(sid, "echo to-out; echo to-err >&2")
	if err != nil {
		t.Fatalf("Exec: %v", err)
	}
	if !strings.Contains(stdout, "to-out") {
		t.Errorf("stdout = %q", stdout)
	}
	if !strings.Contains(stderr, "to-err") {
		t.Errorf("stderr = %q", stderr)
	}
	if code != 0 {
		t.Errorf("code = %d", code)
	}
}

func TestInProcessSandbox_workingDirIsolation(t *testing.T) {
	s := NewInProcessSandbox(0)
	sid1, _ := s.Spawn()
	sid2, _ := s.Spawn()
	defer s.Teardown(sid1)
	defer s.Teardown(sid2)

	// Write a marker file via Exec in sid1
	_, _, _, err := s.Exec(sid1, "touch marker-from-sid1")
	if err != nil {
		t.Fatalf("write marker: %v", err)
	}
	// Confirm sid2 cannot see the marker (separate tempdirs)
	stdout, _, code, err := s.Exec(sid2, "ls marker-from-sid1 2>/dev/null && echo SEEN || echo MISSING")
	if err != nil {
		t.Fatalf("Exec sid2: %v", err)
	}
	if !strings.Contains(stdout, "MISSING") {
		t.Errorf("expected sid2 to NOT see sid1's marker; stdout=%q code=%d", stdout, code)
	}
}

func TestInProcessSandbox_timeoutEnforced(t *testing.T) {
	// 100ms timeout vs 1s sleep → context-deadline exceeded
	s := NewInProcessSandbox(100 * time.Millisecond)
	sid, _ := s.Spawn()
	defer s.Teardown(sid)

	_, _, code, err := s.Exec(sid, "sleep 1; echo done")
	// On timeout, the subprocess is killed; exit-code != 0 OR err set
	if err == nil && code == 0 {
		t.Errorf("expected timeout to manifest as non-zero exit OR err; code=%d err=%v", code, err)
	}
}

func TestInProcessSandbox_teardownRemovesTempDir(t *testing.T) {
	s := NewInProcessSandbox(0)
	sid, _ := s.Spawn()

	// Capture the tempdir
	raw, ok := s.sessions.Load(sid)
	if !ok {
		t.Fatal("session not stored")
	}
	dir := raw.(string)
	if _, err := os.Stat(dir); err != nil {
		t.Fatalf("expected tempdir to exist post-Spawn: %v", err)
	}
	// Drop a marker to confirm cleanup
	_ = os.WriteFile(filepath.Join(dir, "leftover"), []byte("data"), 0o644)

	if err := s.Teardown(sid); err != nil {
		t.Fatalf("Teardown: %v", err)
	}
	if _, err := os.Stat(dir); err == nil {
		t.Errorf("tempdir %s still exists post-Teardown", dir)
	}
}

// ====== DockerSandbox (skip when docker unavailable) ======

func TestDockerSandbox_constructorDefaults(t *testing.T) {
	d := NewDockerSandbox("")
	if d.image != "alpine:latest" {
		t.Errorf("default image = %q, want alpine:latest", d.image)
	}
	d2 := NewDockerSandbox("ubuntu:22.04")
	if d2.image != "ubuntu:22.04" {
		t.Errorf("explicit image = %q", d2.image)
	}
}

func TestDockerSandbox_unknownSessionExec(t *testing.T) {
	d := NewDockerSandbox("")
	if _, _, _, err := d.Exec("never-spawned", "echo x"); err == nil {
		t.Error("expected error for unknown session")
	}
}

func TestDockerSandbox_teardownIdempotent(t *testing.T) {
	d := NewDockerSandbox("")
	if err := d.Teardown("never-spawned"); err != nil {
		t.Errorf("teardown unknown should be idempotent, got: %v", err)
	}
}

func TestDockerSandbox_spawnExecTeardownIntegration(t *testing.T) {
	if !DockerAvailable() {
		t.Skip("docker not available — integration test skipped")
	}
	d := NewDockerSandbox("alpine:latest")
	sid, err := d.Spawn()
	if err != nil {
		t.Fatalf("Spawn: %v", err)
	}
	t.Cleanup(func() { _ = d.Teardown(sid) })

	stdout, _, code, err := d.Exec(sid, "echo hello-from-docker")
	if err != nil {
		t.Fatalf("Exec: %v", err)
	}
	if !strings.Contains(stdout, "hello-from-docker") {
		t.Errorf("docker exec stdout = %q", stdout)
	}
	if code != 0 {
		t.Errorf("exit-code = %d", code)
	}
}

// ====== Interface conformance + helpers ======

func TestSandboxInterface_compileTimeAssertion(t *testing.T) {
	// Compile-time `var _ Sandbox = ...` in sandbox.go covers conformance.
	// Runtime smoke-test verifies both impls can be assigned to the
	// interface variable + invoked.
	var _ Sandbox = NewInProcessSandbox(0)
	var s Sandbox = NewDockerSandbox("alpine:latest")
	// Smoke-test: unknown-session error path doesn't require docker.
	if _, _, _, err := s.Exec("nope", "echo x"); err == nil {
		t.Error("expected error from interface variable invocation")
	}
}

func TestDockerAvailable_returnsBool(t *testing.T) {
	// Smoke-test: returns true or false (no panic). Actual value depends
	// on local environment; both branches valid.
	avail := DockerAvailable()
	t.Logf("DockerAvailable() = %v (informational; both values valid in CI)", avail)
}
