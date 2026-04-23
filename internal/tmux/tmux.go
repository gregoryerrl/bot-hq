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
