package tmux

import (
	"fmt"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

// Session represents a tmux session.
type Session struct {
	Name     string
	Windows  int
	Attached bool
	Created  time.Time
}

// Exec runs a tmux command and returns stdout.
func Exec(args ...string) (string, error) {
	cmd := exec.Command("tmux", args...)
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("tmux %s: %w", strings.Join(args, " "), err)
	}
	return strings.TrimSpace(string(out)), nil
}

// ListSessions lists all tmux sessions.
func ListSessions() ([]Session, error) {
	out, err := Exec("list-sessions", "-F", "#{session_name}\t#{session_windows}\t#{session_attached}\t#{session_created}")
	if err != nil {
		return nil, err
	}
	if out == "" {
		return nil, nil
	}

	var sessions []Session
	for _, line := range strings.Split(out, "\n") {
		parts := strings.SplitN(line, "\t", 4)
		if len(parts) < 4 {
			continue
		}
		windows, _ := strconv.Atoi(parts[1])
		attached := parts[2] == "1"
		createdUnix, _ := strconv.ParseInt(parts[3], 10, 64)
		sessions = append(sessions, Session{
			Name:     parts[0],
			Windows:  windows,
			Attached: attached,
			Created:  time.Unix(createdUnix, 0),
		})
	}
	return sessions, nil
}

// NewSession creates a detached tmux session.
func NewSession(name, cwd string) error {
	args := []string{"new-session", "-d", "-s", name}
	if cwd != "" {
		args = append(args, "-c", cwd)
	}
	_, err := Exec(args...)
	return err
}

// KillSession kills a tmux session by name.
func KillSession(name string) error {
	_, err := Exec("kill-session", "-t", name)
	return err
}

// SendKeys sends keystrokes to a tmux target (session:window.pane).
// Uses -l (literal) flag so key names in content are not interpreted by tmux.
// When enter is true, a small delay is inserted before sending Enter to allow
// the target application's bracketed paste handler to finish processing.
func SendKeys(target, keys string, enter bool) error {
	args := []string{"send-keys", "-t", target, "-l", keys}
	if enter {
		// Enter must be sent as a separate non-literal send-keys call
		if _, err := Exec(args...); err != nil {
			return err
		}
		// Delay for bracketed paste processing (Claude Code, etc.)
		time.Sleep(500 * time.Millisecond)
		_, err := Exec("send-keys", "-t", target, "Enter")
		return err
	}
	_, err := Exec(args...)
	return err
}

// CapturePane captures visible content of a tmux pane.
func CapturePane(target string, lines int) (string, error) {
	start := fmt.Sprintf("-%d", lines)
	return Exec("capture-pane", "-t", target, "-p", "-S", start)
}

// HasTmux checks if tmux is available on the system.
func HasTmux() bool {
	_, err := exec.LookPath("tmux")
	return err == nil
}

// promptByteAnchor is the literal byte sequence for ❯ + NBSP (U+276F +
// U+00A0). Pinned at byte level rather than rune level because:
//   - Claude Code renders the cursor space as a NON-BREAKING SPACE (U+00A0,
//     bytes 0xC2 0xA0), not a regular space (U+0020). NBSP is non-whitespace
//     from tmux capture-pane's line-trim perspective, so the cursor position
//     survives capture even at end-of-visual-line. Empirical hexdump from a
//     live Claude pane confirms `e2 9d af c2 a0`.
//   - Matching at byte level avoids the regex-on-visual-chars failure mode
//     where lipgloss/ANSI rendering could produce visually-identical variant
//     codepoints, AND prevents collision with the regular-space form that
//     appears in chat text / docs / code where humans type `❯ ` literally.
const promptByteAnchor = "\xe2\x9d\xaf\xc2\xa0"

// PromptCheckGrace is the grace window for at-prompt checks that must
// tolerate transient mid-render frames (e.g. claude_message busy detection).
// 750ms balances snappy response with resilience against partial-frame
// false-busy on a Claude pane that's redrawing its input box.
const PromptCheckGrace = 750 * time.Millisecond

// promptCaptureLines is the number of lines to capture when scanning for the
// prompt anchor. Per spec (variable per call site): the prompt can render
// several lines above the literal last pane line because of the input-box
// rules and footer. 30 lines also covers spawn-time MCP loading messages
// above the eventual prompt.
const promptCaptureLines = 30

// promptPollInterval is how often WaitForPrompt re-checks the pane during
// timeout polling. 200ms balances latency against tmux capture-pane shell-out
// cost.
const promptPollInterval = 200 * time.Millisecond

// WaitForPrompt scans the pane for the "❯ " input prompt anchor.
// Returns immediately on detection. With timeout=0, performs a single-shot
// check (use for instantaneous at-prompt queries). With timeout>0, polls
// every promptPollInterval until detected or deadline.
//
// On timeout, returns (false, lastCapture, nil) — error is nil because
// timeout is not exceptional, the absence of a prompt is the answer. The
// returned output is the most recent capture so callers can diagnose what
// was on the pane.
//
// Capture errors during polling propagate immediately as (false, "", err).
//
// Use case mapping:
//   - hub_spawn boot wait: WaitForPrompt(target, 30*time.Second) — wait up
//     to 30s for Claude to finish booting before sending the user prompt.
//     Replaces the brittle time.Sleep(3s) gate (bug #2).
//   - claude_message at-prompt check: WaitForPrompt(target, PromptCheckGrace)
//     — 750ms grace tolerates mid-render frames without false-busy (bug #3).
func WaitForPrompt(target string, timeout time.Duration) (atPrompt bool, output string, err error) {
	if timeout == 0 {
		return checkPromptOnce(target)
	}
	deadline := time.Now().Add(timeout)
	for {
		at, out, err := checkPromptOnce(target)
		if err != nil {
			return false, "", err
		}
		if at {
			return true, out, nil
		}
		if time.Now().After(deadline) {
			return false, out, nil
		}
		time.Sleep(promptPollInterval)
	}
}

func checkPromptOnce(target string) (bool, string, error) {
	out, err := CapturePane(target, promptCaptureLines)
	if err != nil {
		return false, "", err
	}
	return strings.Contains(out, promptByteAnchor), out, nil
}
