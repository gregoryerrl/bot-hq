package toolgate

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// setupGatesEnv writes the 3 gate-files into a tempdir + sets
// BOT_HQ_HOME for R33 to find them. Returns the tempdir + per-class
// SHA256 of the gate-file content.
func setupGatesEnv(t *testing.T) (home string, shas map[ChecklistClass]string) {
	t.Helper()
	home = t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)
	gatesDir := filepath.Join(home, "gates")
	if err := os.MkdirAll(gatesDir, 0o755); err != nil {
		t.Fatalf("mkdir gates: %v", err)
	}
	shas = map[ChecklistClass]string{}
	for _, c := range []ChecklistClass{ClassCommit, ClassPush, ClassMerge} {
		content := []byte(fmt.Sprintf("# %s gate-file (test)\n\ntest content unique per class %s\n", c, c))
		path := filepath.Join(gatesDir, gateFileName(c))
		if err := os.WriteFile(path, content, 0o644); err != nil {
			t.Fatalf("write gate-file %s: %v", c, err)
		}
		sum := sha256.Sum256(content)
		shas[c] = hex.EncodeToString(sum[:])
	}
	return home, shas
}

// writeAgentState writes a minimal AgentState JSON for the given agent
// with pre_<class>_checklist_sha_seen fields populated per the input map.
// shas is a map of class → SHA value to write; missing classes leave the
// field absent. atMsgIDByClass maps class → at_msg_id value.
func writeAgentState(t *testing.T, home, agentID string, shas map[ChecklistClass]string, atMsgIDByClass map[ChecklistClass]int64) {
	t.Helper()
	dir := filepath.Join(home, agentID)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir agent dir: %v", err)
	}
	body := "{\n"
	body += fmt.Sprintf("  \"agent_id\": %q,\n", agentID)
	for c, sha := range shas {
		body += fmt.Sprintf("  \"pre_%s_checklist_sha_seen\": %q,\n", c, sha)
	}
	for c, id := range atMsgIDByClass {
		body += fmt.Sprintf("  \"pre_%s_checklist_sha_seen_at_msg_id\": %d,\n", c, id)
	}
	body += "  \"trailing\": null\n}\n"
	path := filepath.Join(dir, "last_state.json")
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatalf("write agent state: %v", err)
	}
}

// seedHubMessages opens a fresh hub.db at <home>/hub.db and inserts N
// messages from agentID so GetLatestMessageFrom returns msg ID == N.
func seedHubMessages(t *testing.T, home, agentID string, n int) {
	t.Helper()
	dbPath := filepath.Join(home, "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatalf("open hub.db: %v", err)
	}
	defer db.Close()
	for i := 0; i < n; i++ {
		if _, err := db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   "",
			Type:      protocol.MsgUpdate,
			Content:   "seed",
			Created:   time.Now(),
		}); err != nil {
			t.Fatalf("insert seed msg %d: %v", i, err)
		}
	}
}

// TestVerifyChecklistCite covers the R33 verification decision tree
// across all 3 gate-classes + soft-allow / hard-block paths.
func TestVerifyChecklistCite(t *testing.T) {
	t.Run("commit_override_allows", func(t *testing.T) {
		home, _ := setupGatesEnv(t)
		_ = home
		t.Setenv(commitOverrideEnvVar, "1")
		v := VerifyChecklistCite(ClassCommit, "brian", "")
		if !v.Allow || v.SkippedForm != "override" {
			t.Errorf("override path: got Allow=%v SkippedForm=%q, want Allow=true SkippedForm=override", v.Allow, v.SkippedForm)
		}
	})

	t.Run("push_override_env_var_ignored", func(t *testing.T) {
		// Per F9 amend: BRIAN_PRE_COMMIT_GATE_OVERRIDE only affects commit
		// class. Even if it's set, push class must NOT honor it.
		home, shas := setupGatesEnv(t)
		_ = home
		t.Setenv(commitOverrideEnvVar, "1")
		// No AgentState → should still block because override only applies
		// to commit class.
		v := VerifyChecklistCite(ClassPush, "brian", "")
		if v.Allow {
			t.Errorf("push class must NOT honor commit override; got Allow=true")
		}
		_ = shas
	})

	t.Run("gatefile_absent_soft_allow", func(t *testing.T) {
		home := t.TempDir()
		t.Setenv("BOT_HQ_HOME", home)
		// No gates dir created
		v := VerifyChecklistCite(ClassCommit, "brian", "")
		if !v.Allow || v.SkippedForm != "gatefile-absent" {
			t.Errorf("gatefile-absent: got Allow=%v SkippedForm=%q, want Allow=true SkippedForm=gatefile-absent", v.Allow, v.SkippedForm)
		}
	})

	t.Run("commit_footer_match_allows", func(t *testing.T) {
		home, shas := setupGatesEnv(t)
		_ = home
		commitMsg := fmt.Sprintf("subject\n\nbody line\n\nPre-commit-checklist-SHA: %s\n", shas[ClassCommit])
		v := VerifyChecklistCite(ClassCommit, "brian", commitMsg)
		if !v.Allow {
			t.Errorf("commit footer match: got Allow=false reason=%q, want Allow=true", v.Reason)
		}
	})

	t.Run("commit_footer_mismatch_falls_through_to_agentstate", func(t *testing.T) {
		home, shas := setupGatesEnv(t)
		// Footer with WRONG sha
		wrongSHA := "0000000000000000000000000000000000000000000000000000000000000000"
		commitMsg := fmt.Sprintf("subject\n\nPre-commit-checklist-SHA: %s\n", wrongSHA)
		// AgentState has CORRECT sha + fresh at_msg_id
		seedHubMessages(t, home, "brian", 1)
		writeAgentState(t, home, "brian",
			map[ChecklistClass]string{ClassCommit: shas[ClassCommit]},
			map[ChecklistClass]int64{ClassCommit: 1})
		v := VerifyChecklistCite(ClassCommit, "brian", commitMsg)
		if !v.Allow {
			t.Errorf("footer mismatch + agentstate fresh: got Allow=false reason=%q, want Allow=true", v.Reason)
		}
	})

	t.Run("commit_no_footer_no_agentstate_blocks", func(t *testing.T) {
		setupGatesEnv(t)
		v := VerifyChecklistCite(ClassCommit, "brian", "subject only no footer")
		if v.Allow {
			t.Errorf("no footer + no agentstate: got Allow=true, want block; reason: %q", v.Reason)
		}
	})

	t.Run("push_agentstate_fresh_allows", func(t *testing.T) {
		home, shas := setupGatesEnv(t)
		seedHubMessages(t, home, "brian", 3)
		writeAgentState(t, home, "brian",
			map[ChecklistClass]string{ClassPush: shas[ClassPush]},
			map[ChecklistClass]int64{ClassPush: 3})
		v := VerifyChecklistCite(ClassPush, "brian", "")
		if !v.Allow {
			t.Errorf("push agentstate fresh: got Allow=false reason=%q, want Allow=true", v.Reason)
		}
	})

	t.Run("push_agentstate_stale_blocks", func(t *testing.T) {
		home, shas := setupGatesEnv(t)
		// Seed 10 messages; AgentState at_msg_id=1; window default 5; delta 9 > 5
		seedHubMessages(t, home, "brian", 10)
		writeAgentState(t, home, "brian",
			map[ChecklistClass]string{ClassPush: shas[ClassPush]},
			map[ChecklistClass]int64{ClassPush: 1})
		v := VerifyChecklistCite(ClassPush, "brian", "")
		if v.Allow {
			t.Errorf("push agentstate stale: got Allow=true, want block; reason: %q", v.Reason)
		}
	})

	t.Run("push_agentstate_sha_mismatch_blocks", func(t *testing.T) {
		home, _ := setupGatesEnv(t)
		seedHubMessages(t, home, "brian", 1)
		writeAgentState(t, home, "brian",
			map[ChecklistClass]string{ClassPush: "deadbeef" + "0000000000000000000000000000000000000000000000000000000000"},
			map[ChecklistClass]int64{ClassPush: 1})
		v := VerifyChecklistCite(ClassPush, "brian", "")
		if v.Allow {
			t.Errorf("push agentstate sha mismatch: got Allow=true, want block; reason: %q", v.Reason)
		}
	})

	t.Run("merge_agentstate_fresh_allows", func(t *testing.T) {
		home, shas := setupGatesEnv(t)
		seedHubMessages(t, home, "brian", 5)
		writeAgentState(t, home, "brian",
			map[ChecklistClass]string{ClassMerge: shas[ClassMerge]},
			map[ChecklistClass]int64{ClassMerge: 5})
		v := VerifyChecklistCite(ClassMerge, "brian", "")
		if !v.Allow {
			t.Errorf("merge agentstate fresh: got Allow=false reason=%q, want Allow=true", v.Reason)
		}
	})

	t.Run("merge_agentstate_stale_blocks", func(t *testing.T) {
		home, shas := setupGatesEnv(t)
		seedHubMessages(t, home, "brian", 20)
		writeAgentState(t, home, "brian",
			map[ChecklistClass]string{ClassMerge: shas[ClassMerge]},
			map[ChecklistClass]int64{ClassMerge: 5})
		v := VerifyChecklistCite(ClassMerge, "brian", "")
		if v.Allow {
			t.Errorf("merge agentstate stale: got Allow=true, want block; reason: %q", v.Reason)
		}
	})

	t.Run("merge_agentstate_absent_blocks", func(t *testing.T) {
		setupGatesEnv(t)
		v := VerifyChecklistCite(ClassMerge, "brian", "")
		if v.Allow {
			t.Errorf("merge no agentstate: got Allow=true, want block; reason: %q", v.Reason)
		}
	})

	t.Run("freshness_window_env_var_honored", func(t *testing.T) {
		home, shas := setupGatesEnv(t)
		t.Setenv("BRIAN_CHECKLIST_FRESHNESS_WINDOW", "20")
		seedHubMessages(t, home, "brian", 15)
		writeAgentState(t, home, "brian",
			map[ChecklistClass]string{ClassPush: shas[ClassPush]},
			map[ChecklistClass]int64{ClassPush: 1})
		// delta=14, window=20, should pass
		v := VerifyChecklistCite(ClassPush, "brian", "")
		if !v.Allow {
			t.Errorf("env-var window=20: got Allow=false reason=%q, want Allow=true (delta=14 < 20)", v.Reason)
		}
	})
}

// TestRunHook_R33_PreCommit verifies the L-5 R33 pre-commit gate-CHECK
// fires from the hook on git commit and blocks when SHA-cite missing.
func TestRunHook_R33_PreCommit(t *testing.T) {
	home, shas := setupGatesEnv(t)
	// Pre-stage hub.db + AgentState so K-13 R12 can pass when we provide
	// a peer-greenflag footer; otherwise K-13 blocks first and we won't
	// reach R33.
	seedPeerGreenflag(t, home, "rain", 1)
	commitMsgWithoutSHA := "subject\n\nbody\n\npeer-greenflag-msg-id: 1\n"
	cmd := fmt.Sprintf(`git commit -m %q`, commitMsgWithoutSHA)
	input := HookInput{
		ToolName:  "Bash",
		ToolInput: map[string]any{"command": cmd},
	}
	t.Setenv("BOT_HQ_AGENT_ID", "brian")
	code, stderr := runHookInline(t, input)
	if code != ExitBlock {
		t.Errorf("R33 pre-commit no SHA-cite: got exit=%d, want ExitBlock=%d; stderr: %q", code, ExitBlock, stderr)
	}
	if !contains(stderr, "L-5 R33 pre-commit-checklist gate-CHECK") {
		t.Errorf("stderr missing R33 marker: %q", stderr)
	}
	_ = shas
}

// TestRunHook_R33_PrePush verifies the L-5 R33 pre-push gate-CHECK
// fires from the hook on git push and blocks when AgentState absent.
func TestRunHook_R33_PrePush(t *testing.T) {
	home, _ := setupGatesEnv(t)
	_ = home
	cmd := `git push origin main`
	input := HookInput{
		ToolName:  "Bash",
		ToolInput: map[string]any{"command": cmd},
	}
	t.Setenv("BOT_HQ_AGENT_ID", "brian")
	code, stderr := runHookInline(t, input)
	if code != ExitBlock {
		t.Errorf("R33 pre-push no AgentState: got exit=%d, want ExitBlock=%d; stderr: %q", code, ExitBlock, stderr)
	}
	if !contains(stderr, "L-5 R33 pre-push-checklist gate-CHECK") {
		t.Errorf("stderr missing R33 marker: %q", stderr)
	}
}

// TestRunHook_R33_PreMerge verifies the L-5 R33 pre-merge gate-CHECK
// fires from the hook on gh pr merge and blocks when AgentState absent.
func TestRunHook_R33_PreMerge(t *testing.T) {
	home, _ := setupGatesEnv(t)
	_ = home
	cmd := `gh pr merge 42 --squash`
	input := HookInput{
		ToolName:  "Bash",
		ToolInput: map[string]any{"command": cmd},
	}
	t.Setenv("BOT_HQ_AGENT_ID", "brian")
	code, stderr := runHookInline(t, input)
	if code != ExitBlock {
		t.Errorf("R33 pre-merge no AgentState: got exit=%d, want ExitBlock=%d; stderr: %q", code, ExitBlock, stderr)
	}
	if !contains(stderr, "L-5 R33 pre-merge-checklist gate-CHECK") {
		t.Errorf("stderr missing R33 marker: %q", stderr)
	}
}

// seedPeerGreenflag inserts a single greenflag-class message from the
// peer agent, returning msg ID for footer cite.
func seedPeerGreenflag(t *testing.T, home, peerAgent string, msgID int64) {
	t.Helper()
	dbPath := filepath.Join(home, "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatalf("open hub.db: %v", err)
	}
	defer db.Close()
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: peerAgent,
		Type:      protocol.MsgResponse,
		Content:   "BRAIN-AGREED + GREENFLAG on diff",
		Created:   time.Now(),
	}); err != nil {
		t.Fatalf("insert peer greenflag: %v", err)
	}
	_ = msgID
}

// contains is a tiny helper to avoid importing strings in tests that
// only need substring matching.
func contains(s, sub string) bool {
	return len(sub) == 0 || (len(s) >= len(sub) && indexOf(s, sub) >= 0)
}

func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}

// runHookInline runs RunHook against the given input without touching
// BOT_HQ_AGENT_ID (caller is expected to set it via t.Setenv) — the
// existing runHookWithInput helper sets that env var which conflicts with
// R33 tests that need it stable across the test body.
func runHookInline(t *testing.T, input HookInput) (int, string) {
	t.Helper()
	data, err := json.Marshal(input)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	var stderr bytes.Buffer
	code := RunHook(bytes.NewReader(data), &stderr)
	return code, stderr.String()
}
