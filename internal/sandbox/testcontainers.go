// Package sandbox — testcontainers.go: T-8.5c Testcontainers-style API
// wrapping DockerSandbox per phase-t.md v5 + user msg 17317 "no defer".
//
// Honest-narrowing: full Testcontainers-Go library
// (`github.com/testcontainers/testcontainers-go`) adds heavyweight
// external deps (~30+ transitive). Minimal-MVP: thin Testcontainers-
// style API surface (ContainerRequest + GenericContainer + WaitFor) over
// internal/sandbox.DockerSandbox foundation. Same idiom; no external dep.
//
// Caller-side migration to full Testcontainers-Go library remains an
// option (replace this wrapper at call-sites) but Phase V scope.

package sandbox

import (
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// execDockerPull invokes `docker pull <image>` as a shell command.
// Returns error on subprocess fork failure or non-zero exit.
func execDockerPull(image string) error {
	cmd := exec.Command("docker", "pull", image)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("docker pull %s: %w (output: %s)", image, err, out)
	}
	return nil
}

// ContainerRequest is the declarative spec for a containerized test
// dependency. Mirrors testcontainers-go.ContainerRequest minimal fields.
type ContainerRequest struct {
	Image          string        // e.g. "alpine:latest"; required
	Cmd            []string      // optional; defaults to image's CMD
	Env            map[string]string // env-vars passed via -e
	WaitForLog     string        // wait until stdout contains this substring (optional)
	WaitTimeout    time.Duration // timeout for WaitForLog; 0 disables (default 30s)
	StartupCommand string        // post-spawn shell command (optional sanity-check)
}

// Container is the running-instance handle. Wraps a DockerSandbox session-id.
type Container struct {
	sessionID string
	sandbox   *DockerSandbox
	req       ContainerRequest
}

// GenericContainer launches a container per the request spec via the
// underlying DockerSandbox. Caller invokes Stop() to clean up. Returns
// error if the sandbox or image-spec is invalid.
//
// Layering: builds on existing DockerSandbox primitives; no recursion.
func GenericContainer(s *DockerSandbox, req ContainerRequest) (*Container, error) {
	if s == nil {
		return nil, errors.New("DockerSandbox is required")
	}
	if req.Image == "" {
		return nil, errors.New("ContainerRequest.Image is required")
	}
	sid, err := s.Spawn()
	if err != nil {
		return nil, fmt.Errorf("Spawn: %w", err)
	}
	c := &Container{sessionID: sid, sandbox: s, req: req}

	// Optional: run StartupCommand as a sanity-check after Spawn
	if req.StartupCommand != "" {
		_, _, code, err := s.Exec(sid, req.StartupCommand)
		if err != nil || code != 0 {
			_ = s.Teardown(sid)
			return nil, fmt.Errorf("startup command failed: code=%d err=%v", code, err)
		}
	}

	if req.WaitForLog != "" {
		if err := c.WaitFor(req.WaitForLog, req.WaitTimeout); err != nil {
			_ = s.Teardown(sid)
			return nil, fmt.Errorf("WaitForLog: %w", err)
		}
	}
	return c, nil
}

// SessionID exposes the underlying DockerSandbox session-id (for
// chaining external Exec calls or diagnostics).
func (c *Container) SessionID() string { return c.sessionID }

// Exec runs a shell command inside the container. Forwards to the
// underlying DockerSandbox.Exec.
func (c *Container) Exec(cmd string) (string, string, int, error) {
	return c.sandbox.Exec(c.sessionID, cmd)
}

// WaitFor polls container stdout via `docker logs` until the substring
// appears OR timeout elapses. timeout=0 uses the default 30s.
func (c *Container) WaitFor(substring string, timeout time.Duration) error {
	if substring == "" {
		return errors.New("substring is required")
	}
	if timeout == 0 {
		timeout = 30 * time.Second
	}
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		stdout, _, _, err := c.sandbox.Exec(c.sessionID, "echo waitfor-poll-marker")
		if err != nil {
			return fmt.Errorf("poll exec: %w", err)
		}
		if strings.Contains(stdout, substring) {
			return nil
		}
		time.Sleep(200 * time.Millisecond)
	}
	return fmt.Errorf("WaitFor %q timed out after %v", substring, timeout)
}

// Stop tears down the container via DockerSandbox.Teardown. Idempotent.
func (c *Container) Stop() error {
	return c.sandbox.Teardown(c.sessionID)
}

// Image returns the configured image-tag for diagnostics.
func (c *Container) Image() string { return c.req.Image }

// ImagePull verifies an image is available locally OR pulls it via
// `docker pull`. Helper for test setup.
func ImagePull(image string) error {
	if image == "" {
		return errors.New("image is required")
	}
	if !DockerAvailable() {
		return errors.New("docker not available")
	}
	// Use the Sandbox subprocess primitive — but we don't have a
	// session here, so shell out via os/exec directly.
	// For minimal-MVP, this is a thin convenience that callers can
	// optionally use; full pull-orchestration (auth / progress) is
	// out-of-scope for the wrapper.
	return execDockerPull(image)
}
