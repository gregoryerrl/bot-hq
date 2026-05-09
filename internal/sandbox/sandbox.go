// Package sandbox implements isolated execution environments per
// phase-t.md v5 T-4 + T-8.5a. Verify primitives invoke these to execute
// tests / repro browser interactions without polluting the host.
//
// Two implementations:
//
//	DockerSandbox    — production-class isolation via docker CLI subprocess
//	InProcessSandbox — baseline using t.TempDir + os/exec; NO process or
//	                   filesystem isolation; fallback when Docker is
//	                   unavailable + unit-test target for the interface
//
// Future deliverables (T-8.5b/c follow-up):
//
//	Playwright-Go integration for browser-driven UI tests
//	Testcontainers-Go integration if CLI-wrap proves insufficient

package sandbox

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
)

// Sandbox is the contract DockerSandbox + InProcessSandbox both satisfy.
// Mirrors verify.Sandbox (internal/verify/verify.go:265) duck-typed for
// future-consolidation. Caller-side adapters bridge if the interface
// signatures diverge.
type Sandbox interface {
	Spawn() (string, error)
	Exec(sessionID, cmd string) (stdout string, stderr string, exitCode int, err error)
	Teardown(sessionID string) error
}

// DockerAvailable returns true when the `docker` CLI is on PATH AND the
// daemon responds to `docker version`. Used as the integration-test
// skip-guard so CI without Docker passes.
func DockerAvailable() bool {
	if _, err := exec.LookPath("docker"); err != nil {
		return false
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "docker", "version", "--format", "{{.Server.Version}}")
	return cmd.Run() == nil
}

// ====== DockerSandbox ======

// DockerSandbox provisions ephemeral docker containers + executes shell
// commands inside via `docker exec`. Containers run `tail -f /dev/null`
// to keep them alive between Exec calls; teardown removes them via
// `docker rm -f`.
type DockerSandbox struct {
	image    string
	sessions sync.Map // sessionID → containerID
}

// NewDockerSandbox constructs a sandbox with the given image. Image
// defaults to "alpine:latest" if empty (small + fast).
func NewDockerSandbox(image string) *DockerSandbox {
	if image == "" {
		image = "alpine:latest"
	}
	return &DockerSandbox{image: image}
}

// Spawn starts a fresh container + returns a session-id mapping to
// the underlying container-id.
func (d *DockerSandbox) Spawn() (string, error) {
	sessionID := uuid.New().String()
	name := "bot-hq-sandbox-" + sessionID[:8]
	out, err := exec.Command("docker", "run", "-d", "--rm", "--name", name, d.image, "tail", "-f", "/dev/null").Output()
	if err != nil {
		return "", fmt.Errorf("docker run %s: %w", d.image, err)
	}
	containerID := strings.TrimSpace(string(out))
	d.sessions.Store(sessionID, containerID)
	return sessionID, nil
}

// Exec runs a shell command inside the container. Returns stdout +
// stderr separately + the exit-code (0 on success, non-zero on command
// failure). A non-zero exit is captured into exitCode + err=nil; only
// hard errors (subprocess fork failure, container missing) return err.
func (d *DockerSandbox) Exec(sessionID, cmdStr string) (string, string, int, error) {
	raw, ok := d.sessions.Load(sessionID)
	if !ok {
		return "", "", -1, fmt.Errorf("unknown session %s", sessionID)
	}
	containerID := raw.(string)
	c := exec.Command("docker", "exec", containerID, "sh", "-c", cmdStr)
	var stdout, stderr strings.Builder
	c.Stdout = &stdout
	c.Stderr = &stderr
	err := c.Run()
	exitCode := 0
	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			exitCode = exitErr.ExitCode()
			err = nil
		} else {
			return stdout.String(), stderr.String(), -1, fmt.Errorf("docker exec: %w", err)
		}
	}
	return stdout.String(), stderr.String(), exitCode, err
}

// Teardown stops + removes the container. Idempotent — no error on
// unknown session-id.
func (d *DockerSandbox) Teardown(sessionID string) error {
	raw, ok := d.sessions.LoadAndDelete(sessionID)
	if !ok {
		return nil
	}
	containerID := raw.(string)
	if err := exec.Command("docker", "rm", "-f", containerID).Run(); err != nil {
		return fmt.Errorf("docker rm %s: %w", containerID, err)
	}
	return nil
}

// ====== InProcessSandbox ======

// InProcessSandbox is a baseline using t.TempDir-style temp directories
// + os/exec subprocesses. NO process or filesystem isolation — used as
// fallback when Docker is unavailable + as the unit-test target for the
// Sandbox interface itself.
//
// NOTE: Not a real "sandbox" in the security sense — minimal isolation.
// Production isolation requires DockerSandbox or future Testcontainers-Go.
type InProcessSandbox struct {
	sessions sync.Map // sessionID → tempdir
	timeout  time.Duration
}

// NewInProcessSandbox constructs the baseline impl. timeout=0 disables
// per-Exec timeouts (caller is responsible for cancellation).
func NewInProcessSandbox(timeout time.Duration) *InProcessSandbox {
	return &InProcessSandbox{timeout: timeout}
}

// Spawn creates a fresh temp directory + records it under a session-id.
func (s *InProcessSandbox) Spawn() (string, error) {
	sessionID := uuid.New().String()
	dir, err := os.MkdirTemp("", "bot-hq-sandbox-")
	if err != nil {
		return "", fmt.Errorf("mkdir temp: %w", err)
	}
	s.sessions.Store(sessionID, dir)
	return sessionID, nil
}

// Exec runs `/bin/sh -c <cmd>` in the session's tempdir. Same exit-code
// semantics as DockerSandbox.Exec.
func (s *InProcessSandbox) Exec(sessionID, cmdStr string) (string, string, int, error) {
	raw, ok := s.sessions.Load(sessionID)
	if !ok {
		return "", "", -1, fmt.Errorf("unknown session %s", sessionID)
	}
	dir := raw.(string)

	var c *exec.Cmd
	if s.timeout > 0 {
		ctx, cancel := context.WithTimeout(context.Background(), s.timeout)
		defer cancel()
		c = exec.CommandContext(ctx, "/bin/sh", "-c", cmdStr)
	} else {
		c = exec.Command("/bin/sh", "-c", cmdStr)
	}
	c.Dir = dir
	var stdout, stderr strings.Builder
	c.Stdout = &stdout
	c.Stderr = &stderr
	err := c.Run()
	exitCode := 0
	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			exitCode = exitErr.ExitCode()
			err = nil
		} else {
			return stdout.String(), stderr.String(), -1, fmt.Errorf("exec: %w", err)
		}
	}
	return stdout.String(), stderr.String(), exitCode, err
}

// Teardown removes the tempdir. Idempotent on unknown session-id.
func (s *InProcessSandbox) Teardown(sessionID string) error {
	raw, ok := s.sessions.LoadAndDelete(sessionID)
	if !ok {
		return nil
	}
	dir := raw.(string)
	return os.RemoveAll(dir)
}

// Compile-time conformance assertions.
var (
	_ Sandbox = (*DockerSandbox)(nil)
	_ Sandbox = (*InProcessSandbox)(nil)
)
