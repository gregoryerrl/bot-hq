package playwright

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func newTestDriver(t *testing.T) *PlaywrightCLI {
	t.Helper()
	d, err := New(filepath.Join(t.TempDir(), "playwright"), 0)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	return d
}

func TestNew_validation(t *testing.T) {
	if _, err := New("", 0); err == nil {
		t.Error("expected error for empty workDir")
	}
}

func TestNew_createsWorkDirIfMissing(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "fresh", "workdir")
	d, err := New(dir, 0)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	if d.WorkDir() != dir {
		t.Errorf("WorkDir = %q, want %q", d.WorkDir(), dir)
	}
	info, err := os.Stat(dir)
	if err != nil {
		t.Fatalf("workDir not created: %v", err)
	}
	if !info.IsDir() {
		t.Error("workDir is not a directory")
	}
}

func TestRunScript_emptyPathRejected(t *testing.T) {
	d := newTestDriver(t)
	if _, _, _, err := d.RunScript(""); err == nil {
		t.Error("expected error for empty scriptPath")
	}
}

func TestRunScript_nonexistentPathRejected(t *testing.T) {
	d := newTestDriver(t)
	if _, _, _, err := d.RunScript("/nonexistent/path/xyz.js"); err == nil {
		t.Error("expected error for nonexistent script")
	}
}

func TestRunScript_nodeAvailableExecutesSimpleScript(t *testing.T) {
	if _, err := os.Stat("/usr/local/bin/node"); err != nil {
		if _, err := os.Stat("/usr/bin/node"); err != nil {
			t.Skip("node not available — skipping script-execution test")
		}
	}
	d := newTestDriver(t)
	scriptPath := filepath.Join(d.WorkDir(), "smoke.js")
	if err := os.WriteFile(scriptPath, []byte(`console.log("hello-from-node")`), 0o600); err != nil {
		t.Fatalf("write smoke script: %v", err)
	}
	stdout, _, code, err := d.RunScript(scriptPath)
	if err != nil {
		t.Fatalf("RunScript: %v", err)
	}
	if code != 0 {
		t.Errorf("exit-code = %d, want 0", code)
	}
	if !strings.Contains(stdout, "hello-from-node") {
		t.Errorf("stdout = %q", stdout)
	}
}

func TestNavigate_emptyURLRejected(t *testing.T) {
	d := newTestDriver(t)
	if err := d.Navigate(""); err == nil {
		t.Error("expected error for empty URL")
	}
}

func TestNavigate_skipsWhenPlaywrightUnavailable(t *testing.T) {
	if PlaywrightAvailable() {
		t.Skip("Playwright available — skipping unavailable-path test")
	}
	d := newTestDriver(t)
	if err := d.Navigate("about:blank"); err == nil {
		t.Error("expected error when playwright unavailable")
	}
}

func TestScreenshot_emptyPathRejected(t *testing.T) {
	d := newTestDriver(t)
	if err := d.Screenshot(""); err == nil {
		t.Error("expected error for empty savePath")
	}
}

func TestPlaywrightAvailable_returnsBool(t *testing.T) {
	avail := PlaywrightAvailable()
	t.Logf("PlaywrightAvailable() = %v (informational; both values valid)", avail)
}

func TestRunScript_timeoutEnforced(t *testing.T) {
	if _, err := os.Stat("/usr/local/bin/node"); err != nil {
		if _, err := os.Stat("/usr/bin/node"); err != nil {
			t.Skip("node not available — skipping timeout test")
		}
	}
	d, _ := New(filepath.Join(t.TempDir(), "pw"), 100*time.Millisecond)
	scriptPath := filepath.Join(d.WorkDir(), "sleep.js")
	_ = os.WriteFile(scriptPath, []byte(`setTimeout(() => {}, 5000)`), 0o600)
	_, _, code, err := d.RunScript(scriptPath)
	// Timeout should manifest as non-zero exit OR err
	if err == nil && code == 0 {
		t.Errorf("expected timeout to non-zero exit OR err; code=%d err=%v", code, err)
	}
}

func TestInterfaceConformance(t *testing.T) {
	// Compile-time `var _ BrowserAutomation = (*PlaywrightCLI)(nil)` in
	// playwright.go covers conformance; runtime smoke just confirms type.
	var b BrowserAutomation = newTestDriver(t)
	if err := b.Navigate(""); err == nil {
		t.Error("expected error from interface variable invocation")
	}
}

func TestJSStringQuoting(t *testing.T) {
	cases := []struct{ in, want string }{
		{`hello`, `"hello"`},
		{`with "quotes"`, `"with \"quotes\""`},
		{`back\slash`, `"back\\slash"`},
		{"line\nbreak", `"line\nbreak"`},
	}
	for _, c := range cases {
		t.Run(c.in, func(t *testing.T) {
			if got := jsString(c.in); got != c.want {
				t.Errorf("jsString(%q) = %q, want %q", c.in, got, c.want)
			}
		})
	}
}
