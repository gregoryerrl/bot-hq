package tmux

import (
	"fmt"
	"os/exec"
	"strconv"
	"strings"
)

// Pane represents a tmux pane with its metadata.
type Pane struct {
	Target  string // session:window.pane
	PID     int
	Command string
	CWD     string
}

// DiscoveredSession represents a Claude Code session found in tmux.
type DiscoveredSession struct {
	TmuxTarget string `json:"tmux_target"`
	PID        int    `json:"pid"`
	CWD        string `json:"cwd"`
}

// ListPanes returns all tmux panes with their metadata.
func ListPanes() ([]Pane, error) {
	out, err := Exec("list-panes", "-a", "-F",
		"#{session_name}:#{window_index}.#{pane_index}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}")
	if err != nil {
		return nil, err
	}
	if out == "" {
		return nil, nil
	}

	var panes []Pane
	for _, line := range strings.Split(out, "\n") {
		parts := strings.SplitN(line, "\t", 4)
		if len(parts) < 4 {
			continue
		}
		pid, _ := strconv.Atoi(parts[1])
		panes = append(panes, Pane{
			Target:  parts[0],
			PID:     pid,
			Command: parts[2],
			CWD:     parts[3],
		})
	}
	return panes, nil
}

// DiscoverClaudeSessions scans all tmux panes and finds running Claude Code sessions.
func DiscoverClaudeSessions() ([]DiscoveredSession, error) {
	panes, err := ListPanes()
	if err != nil {
		return nil, err
	}

	var sessions []DiscoveredSession
	for _, pane := range panes {
		if isClaudeSession(pane) {
			claudePID := findClaudePID(pane.PID)
			if claudePID == 0 {
				claudePID = pane.PID
			}
			sessions = append(sessions, DiscoveredSession{
				TmuxTarget: pane.Target,
				PID:        claudePID,
				CWD:        pane.CWD,
			})
		}
	}
	return sessions, nil
}

// isClaudeSession checks if a tmux pane is running Claude Code.
func isClaudeSession(pane Pane) bool {
	// Check the pane's current command
	if strings.Contains(strings.ToLower(pane.Command), "claude") {
		return true
	}

	// Check child processes for claude
	children := getChildProcesses(pane.PID)
	if strings.Contains(strings.ToLower(children), "claude") {
		return true
	}

	// Check pane output for Claude Code indicators
	output, err := CapturePane(pane.Target, 5)
	if err != nil {
		return false
	}
	lower := strings.ToLower(output)
	return strings.Contains(lower, "claude") || strings.Contains(lower, "claude code")
}

// getChildProcesses returns a string of child process names for the given PID.
func getChildProcesses(pid int) string {
	cmd := exec.Command("pgrep", "-P", fmt.Sprintf("%d", pid), "-l")
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	return string(out)
}

// findClaudePID finds the PID of a claude process that is a child of parentPID.
func findClaudePID(parentPID int) int {
	cmd := exec.Command("sh", "-c",
		fmt.Sprintf("pgrep -P %d -f claude 2>/dev/null | head -1", parentPID))
	out, err := cmd.Output()
	if err != nil {
		return 0
	}
	pid, err := strconv.Atoi(strings.TrimSpace(string(out)))
	if err != nil {
		return 0
	}
	return pid
}
