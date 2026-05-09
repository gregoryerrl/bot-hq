package sandbox

import (
	"strings"
	"testing"
	"time"
)

// ====== ContainerRequest validation ======

func TestGenericContainer_nilSandboxRejected(t *testing.T) {
	_, err := GenericContainer(nil, ContainerRequest{Image: "alpine:latest"})
	if err == nil {
		t.Error("expected error for nil sandbox")
	}
}

func TestGenericContainer_emptyImageRejected(t *testing.T) {
	d := NewDockerSandbox("")
	_, err := GenericContainer(d, ContainerRequest{Image: ""})
	if err == nil {
		t.Error("expected error for empty image")
	}
}

// ====== Container API surface ======

func TestContainer_imageGetter(t *testing.T) {
	c := &Container{req: ContainerRequest{Image: "ubuntu:22.04"}}
	if c.Image() != "ubuntu:22.04" {
		t.Errorf("Image() = %q, want ubuntu:22.04", c.Image())
	}
}

func TestContainer_sessionIDGetter(t *testing.T) {
	c := &Container{sessionID: "test-session-xyz"}
	if c.SessionID() != "test-session-xyz" {
		t.Errorf("SessionID() = %q", c.SessionID())
	}
}

// ====== ImagePull ======

func TestImagePull_emptyImageRejected(t *testing.T) {
	if err := ImagePull(""); err == nil {
		t.Error("expected error for empty image")
	}
}

func TestImagePull_skipsWhenDockerUnavailable(t *testing.T) {
	if DockerAvailable() {
		t.Skip("docker available — skipping unavailable-path test")
	}
	if err := ImagePull("alpine:latest"); err == nil {
		t.Error("expected error when docker unavailable")
	}
}

// ====== WaitFor ======

func TestContainer_waitForEmptySubstringRejected(t *testing.T) {
	c := &Container{}
	if err := c.WaitFor("", 1*time.Second); err == nil {
		t.Error("expected error for empty substring")
	}
}

// ====== Integration (skip when docker unavailable) ======

func TestGenericContainer_spawnExecStopIntegration(t *testing.T) {
	if !DockerAvailable() {
		t.Skip("docker not available — integration test skipped")
	}
	d := NewDockerSandbox("alpine:latest")
	c, err := GenericContainer(d, ContainerRequest{Image: "alpine:latest"})
	if err != nil {
		t.Fatalf("GenericContainer: %v", err)
	}
	t.Cleanup(func() { _ = c.Stop() })

	stdout, _, code, err := c.Exec("echo from-container")
	if err != nil {
		t.Fatalf("Exec: %v", err)
	}
	if code != 0 {
		t.Errorf("exit-code = %d", code)
	}
	if !strings.Contains(stdout, "from-container") {
		t.Errorf("stdout = %q", stdout)
	}
}

func TestGenericContainer_startupCommandFailureCleansUpIntegration(t *testing.T) {
	if !DockerAvailable() {
		t.Skip("docker not available")
	}
	d := NewDockerSandbox("alpine:latest")
	_, err := GenericContainer(d, ContainerRequest{
		Image:          "alpine:latest",
		StartupCommand: "exit 99", // forces non-zero exit
	})
	if err == nil {
		t.Error("expected error from startup-command exit-99")
	}
	if !strings.Contains(err.Error(), "startup command") {
		t.Errorf("err = %v, want containing 'startup command'", err)
	}
}

// ====== Helper compile-test ======

func TestContainerRequest_zeroValueValid(t *testing.T) {
	// ContainerRequest must be constructable as zero-value; only Image
	// is required at GenericContainer-call-time.
	var req ContainerRequest
	if req.Image != "" {
		t.Error("zero-value Image should be empty")
	}
	if req.WaitTimeout != 0 {
		t.Error("zero-value WaitTimeout should be 0")
	}
}
