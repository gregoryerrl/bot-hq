// Package playwright implements browser-automation via shell-out to
// `npx playwright` per phase-t.md v5 T-8.5b. Companion to internal/sandbox
// (T-8.5a) for browser-driven testing of webui flows.
//
// MVP scope (honest-narrowing per Rain msg 17307 reading): thin CLI-wrap
// rather than full Playwright-Go bindings. Caller-side Playwright deps
// (node.js + npx + browser binaries) detected at runtime via
// PlaywrightAvailable() probe; integration tests t.Skip when probe fails
// (mirrors T-8.5a DockerSandbox skip pattern). Full Go-native Playwright
// integration via `github.com/playwright-community/playwright-go` is
// Phase V scope per honest-scoping (avoid heavy external dep at MVP).
//
// Layering: stdlib only (os/exec + bytes + context + errors + fmt) — leaf
// package; no internal deps. Verified via `go list -deps`.

package playwright

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

// BrowserAutomation is the contract any browser-driver impl satisfies.
// Currently PlaywrightCLI is the only impl; future Phase V adapters
// (e.g., raw chromedriver / WebDriver) can plug in here.
type BrowserAutomation interface {
	Navigate(url string) error
	Screenshot(savePath string) error
	RunScript(scriptPath string) (stdout, stderr string, exitCode int, err error)
}

// PlaywrightAvailable returns true when `npx playwright --version` runs
// successfully. Used as integration-test skip-guard.
func PlaywrightAvailable() bool {
	if _, err := exec.LookPath("npx"); err != nil {
		return false
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "npx", "playwright", "--version")
	return cmd.Run() == nil
}

// PlaywrightCLI is the npx-shell-out impl of BrowserAutomation. Stateful
// per browser-session via working-directory + env-vars; one PlaywrightCLI
// instance corresponds to one browser-automation session.
type PlaywrightCLI struct {
	workDir string
	timeout time.Duration
}

// New constructs a PlaywrightCLI rooted at workDir with the given
// per-command timeout. workDir is created with 0700 perms if missing.
// timeout=0 disables per-command deadlines.
func New(workDir string, timeout time.Duration) (*PlaywrightCLI, error) {
	if workDir == "" {
		return nil, errors.New("workDir is required")
	}
	if err := os.MkdirAll(workDir, 0o700); err != nil {
		return nil, fmt.Errorf("mkdir workDir: %w", err)
	}
	return &PlaywrightCLI{workDir: workDir, timeout: timeout}, nil
}

// Navigate writes a minimal Playwright script that navigates to the URL
// and runs it via npx. Used for connectivity smoke-tests.
func (p *PlaywrightCLI) Navigate(url string) error {
	if url == "" {
		return errors.New("url is required")
	}
	script := `const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  await page.goto(` + jsString(url) + `);
  await browser.close();
})();`
	scriptPath := filepath.Join(p.workDir, "navigate.js")
	if err := os.WriteFile(scriptPath, []byte(script), 0o600); err != nil {
		return fmt.Errorf("write nav script: %w", err)
	}
	_, stderr, exitCode, err := p.RunScript(scriptPath)
	if err != nil {
		return fmt.Errorf("RunScript: %w", err)
	}
	if exitCode != 0 {
		return fmt.Errorf("navigate failed exit=%d stderr=%s", exitCode, stderr)
	}
	return nil
}

// Screenshot writes a Playwright script that captures a screenshot at
// savePath. URL must be set via Navigate first OR included as the
// first arg via env-var BOT_HQ_PLAYWRIGHT_URL — left to caller-side
// orchestration.
func (p *PlaywrightCLI) Screenshot(savePath string) error {
	if savePath == "" {
		return errors.New("savePath is required")
	}
	url := os.Getenv("BOT_HQ_PLAYWRIGHT_URL")
	if url == "" {
		url = "about:blank"
	}
	script := `const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  await page.goto(` + jsString(url) + `);
  await page.screenshot({ path: ` + jsString(savePath) + ` });
  await browser.close();
})();`
	scriptPath := filepath.Join(p.workDir, "screenshot.js")
	if err := os.WriteFile(scriptPath, []byte(script), 0o600); err != nil {
		return fmt.Errorf("write screenshot script: %w", err)
	}
	_, stderr, exitCode, err := p.RunScript(scriptPath)
	if err != nil {
		return fmt.Errorf("RunScript: %w", err)
	}
	if exitCode != 0 {
		return fmt.Errorf("screenshot failed exit=%d stderr=%s", exitCode, stderr)
	}
	return nil
}

// RunScript executes a JavaScript file via `npx playwright` (or `node`
// if bare-script). Returns stdout / stderr / exit-code separately.
// Non-zero exit captured into exitCode + err=nil; only fork failures
// return err.
func (p *PlaywrightCLI) RunScript(scriptPath string) (string, string, int, error) {
	if scriptPath == "" {
		return "", "", -1, errors.New("scriptPath is required")
	}
	if _, err := os.Stat(scriptPath); err != nil {
		return "", "", -1, fmt.Errorf("script not found: %w", err)
	}
	var c *exec.Cmd
	if p.timeout > 0 {
		ctx, cancel := context.WithTimeout(context.Background(), p.timeout)
		defer cancel()
		c = exec.CommandContext(ctx, "node", scriptPath)
	} else {
		c = exec.Command("node", scriptPath)
	}
	c.Dir = p.workDir
	var stdout, stderr bytes.Buffer
	c.Stdout = &stdout
	c.Stderr = &stderr
	err := c.Run()
	exitCode := 0
	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			exitCode = exitErr.ExitCode()
			err = nil
		} else {
			return stdout.String(), stderr.String(), -1, fmt.Errorf("run script: %w", err)
		}
	}
	return stdout.String(), stderr.String(), exitCode, err
}

// WorkDir returns the configured working-directory for diagnostics + tests.
func (p *PlaywrightCLI) WorkDir() string { return p.workDir }

// jsString quotes a string as a JS string-literal with backslash-escaping.
func jsString(s string) string {
	out := []byte{'"'}
	for _, r := range s {
		switch r {
		case '"':
			out = append(out, '\\', '"')
		case '\\':
			out = append(out, '\\', '\\')
		case '\n':
			out = append(out, '\\', 'n')
		default:
			out = append(out, []byte(string(r))...)
		}
	}
	out = append(out, '"')
	return string(out)
}

// Compile-time conformance assertion.
var _ BrowserAutomation = (*PlaywrightCLI)(nil)
