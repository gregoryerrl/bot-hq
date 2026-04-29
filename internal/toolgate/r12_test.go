package toolgate

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// setupHubDB creates a fresh hub.db at the BOT_HQ_HOME location and
// returns the open handle (for inserting fixtures). Caller closes.
func setupHubDB(t *testing.T) *hub.DB {
	t.Helper()
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)
	dbPath := filepath.Join(tmp, "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatalf("OpenDB: %v", err)
	}
	return db
}

// insertGreenflagMsg inserts a hub message + returns its ID. Used by
// VerifyCommit tests to seed peer-greenflag fixtures.
func insertGreenflagMsg(t *testing.T, db *hub.DB, fromAgent, content string, age time.Duration) int64 {
	t.Helper()
	msg := protocol.Message{
		FromAgent: fromAgent,
		ToAgent:   "",
		Type:      protocol.MsgUpdate,
		Content:   content,
		Created:   time.Now().Add(-age),
	}
	id, err := db.InsertMessage(msg)
	if err != nil {
		t.Fatalf("InsertMessage: %v", err)
	}
	return id
}

// TestIsCommitPattern_Positive locks `git commit` first-2-tokens detection.
func TestIsCommitPattern_Positive(t *testing.T) {
	cases := []string{
		`git commit -m "msg"`,
		`git commit --amend`,
		`git commit -F /tmp/msg.txt`,
		`git commit --message="msg"`,
	}
	for _, cmd := range cases {
		if !IsCommitPattern(cmd) {
			t.Errorf("IsCommitPattern(%q) = false, want true", cmd)
		}
	}
}

// TestIsCommitPattern_Negative locks resistance to non-commit + substring.
func TestIsCommitPattern_Negative(t *testing.T) {
	cases := []string{
		`git status`,
		`git push origin main`,
		`echo "git commit"`,
		`cat /tmp/git-commit.log`,
	}
	for _, cmd := range cases {
		if IsCommitPattern(cmd) {
			t.Errorf("IsCommitPattern(%q) = true, want false", cmd)
		}
	}
}

// TestVerifyCommit_BypassOverride locks BRIAN_R12_OVERRIDE=1 → soft-allow.
func TestVerifyCommit_BypassOverride(t *testing.T) {
	t.Setenv("BRIAN_R12_OVERRIDE", "1")
	v := VerifyCommit(`git commit -m "no footer"`, "brian")
	if !v.Allow {
		t.Errorf("BRIAN_R12_OVERRIDE=1: expected Allow=true; got false (reason=%s)", v.Reason)
	}
	if v.SkippedForm != "override" {
		t.Errorf("expected SkippedForm=override; got %q", v.SkippedForm)
	}
}

// TestVerifyCommit_FormCoverage locks soft-allow on unsupported forms.
func TestVerifyCommit_FormCoverage(t *testing.T) {
	cases := []struct {
		name        string
		command     string
		wantSkipped string
	}{
		{"editor-form-no-flag", `git commit`, "editor-form"},
		{"amend-form", `git commit --amend`, "amend"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			v := VerifyCommit(tc.command, "brian")
			if !v.Allow {
				t.Errorf("expected Allow=true on unsupported form; got false (reason=%s)", v.Reason)
			}
			if v.SkippedForm != tc.wantSkipped {
				t.Errorf("expected SkippedForm=%q; got %q", tc.wantSkipped, v.SkippedForm)
			}
		})
	}
}

// TestVerifyCommit_MissingFooter locks block on commit-msg without footer.
func TestVerifyCommit_MissingFooter(t *testing.T) {
	db := setupHubDB(t)
	defer db.Close()

	v := VerifyCommit(`git commit -m "no footer here"`, "brian")
	if v.Allow {
		t.Errorf("expected Allow=false on missing footer; got true")
	}
	if v.Reason == "" {
		t.Errorf("expected non-empty Reason on block")
	}
}

// TestVerifyCommit_FooterReferencesNonexistentMsg locks block when cited
// msg-id doesn't exist in hub.db.
func TestVerifyCommit_FooterReferencesNonexistentMsg(t *testing.T) {
	db := setupHubDB(t)
	defer db.Close()

	commitMsg := "fix: something\n\npeer-greenflag-msg-id: 99999"
	tmpFile := filepath.Join(t.TempDir(), "msg.txt")
	if err := os.WriteFile(tmpFile, []byte(commitMsg), 0o644); err != nil {
		t.Fatalf("write msg file: %v", err)
	}

	v := VerifyCommit(`git commit -F `+tmpFile, "brian")
	if v.Allow {
		t.Errorf("expected Allow=false on nonexistent msg-id; got true")
	}
}

// TestVerifyCommit_FooterValidGreenflagFromPeer locks happy path:
// footer cites peer msg with greenflag content, within window → allow.
func TestVerifyCommit_FooterValidGreenflagFromPeer(t *testing.T) {
	db := setupHubDB(t)
	defer db.Close()

	// Insert peer-greenflag msg from rain
	id := insertGreenflagMsg(t, db, "rain", "BRAIN-AGREED on K-X impl shape; greenflag for commit", 5*time.Minute)

	commitMsg := "K-X: feature impl\n\npeer-greenflag-msg-id: " + intToStr(id)
	tmpFile := filepath.Join(t.TempDir(), "msg.txt")
	if err := os.WriteFile(tmpFile, []byte(commitMsg), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	v := VerifyCommit(`git commit -F `+tmpFile, "brian")
	if !v.Allow {
		t.Errorf("expected Allow=true on valid peer greenflag; got false (reason=%s)", v.Reason)
	}
}

// TestVerifyCommit_FooterFromSelf locks block when cited msg is from
// the committer (self-greenflag must come from PEER, not self).
func TestVerifyCommit_FooterFromSelf(t *testing.T) {
	db := setupHubDB(t)
	defer db.Close()

	id := insertGreenflagMsg(t, db, "brian", "BRAIN-AGREED self-vouching nope", 5*time.Minute)

	commitMsg := "fix: x\n\npeer-greenflag-msg-id: " + intToStr(id)
	tmpFile := filepath.Join(t.TempDir(), "msg.txt")
	os.WriteFile(tmpFile, []byte(commitMsg), 0o644)

	v := VerifyCommit(`git commit -F `+tmpFile, "brian")
	if v.Allow {
		t.Errorf("expected Allow=false on self-greenflag; got true")
	}
}

// TestVerifyCommit_FooterOutsideWindow locks block when cited msg is
// older than the recency window (default 60min).
func TestVerifyCommit_FooterOutsideWindow(t *testing.T) {
	db := setupHubDB(t)
	defer db.Close()

	id := insertGreenflagMsg(t, db, "rain", "BRAIN-AGREED stale", 90*time.Minute)

	commitMsg := "fix\n\npeer-greenflag-msg-id: " + intToStr(id)
	tmpFile := filepath.Join(t.TempDir(), "msg.txt")
	os.WriteFile(tmpFile, []byte(commitMsg), 0o644)

	v := VerifyCommit(`git commit -F `+tmpFile, "brian")
	if v.Allow {
		t.Errorf("expected Allow=false on stale msg (outside 60min window); got true")
	}
}

// TestVerifyCommit_FooterContentLacksGreenflag locks block when cited
// peer msg exists + within window but content lacks greenflag pattern.
func TestVerifyCommit_FooterContentLacksGreenflag(t *testing.T) {
	db := setupHubDB(t)
	defer db.Close()

	id := insertGreenflagMsg(t, db, "rain", "just chatting; no greenflag intent", 5*time.Minute)

	commitMsg := "fix\n\npeer-greenflag-msg-id: " + intToStr(id)
	tmpFile := filepath.Join(t.TempDir(), "msg.txt")
	os.WriteFile(tmpFile, []byte(commitMsg), 0o644)

	v := VerifyCommit(`git commit -F `+tmpFile, "brian")
	if v.Allow {
		t.Errorf("expected Allow=false on content lacking greenflag; got true")
	}
}

// TestVerifyCommit_HubDBAbsent locks fail-soft: hub.db missing → soft-allow.
func TestVerifyCommit_HubDBAbsent(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("BOT_HQ_HOME", tmp)
	// Note: do NOT create hub.db; absent is the test condition.

	commitMsg := "fix\n\npeer-greenflag-msg-id: 1"
	tmpFile := filepath.Join(t.TempDir(), "msg.txt")
	os.WriteFile(tmpFile, []byte(commitMsg), 0o644)

	v := VerifyCommit(`git commit -F `+tmpFile, "brian")
	if !v.Allow {
		t.Errorf("expected Allow=true on hub.db absent (fail-soft); got false (reason=%s)", v.Reason)
	}
	if v.SkippedForm != "hubdb-absent" {
		t.Errorf("expected SkippedForm=hubdb-absent; got %q", v.SkippedForm)
	}
}

// TestVerifyCommit_RecencyWindowEnvVar locks R12_GREENFLAG_WINDOW_MIN
// env var override. Set to 120min; insert msg aged 90min → allow
// (within extended window even though over default 60min).
func TestVerifyCommit_RecencyWindowEnvVar(t *testing.T) {
	t.Setenv("R12_GREENFLAG_WINDOW_MIN", "120")
	db := setupHubDB(t)
	defer db.Close()

	id := insertGreenflagMsg(t, db, "rain", "BRAIN-AGREED still within extended window", 90*time.Minute)

	commitMsg := "fix\n\npeer-greenflag-msg-id: " + intToStr(id)
	tmpFile := filepath.Join(t.TempDir(), "msg.txt")
	os.WriteFile(tmpFile, []byte(commitMsg), 0o644)

	v := VerifyCommit(`git commit -F `+tmpFile, "brian")
	if !v.Allow {
		t.Errorf("expected Allow=true with extended window 120min; got false (reason=%s)", v.Reason)
	}
}

// intToStr is a tiny helper to convert int64 to decimal string.
func intToStr(n int64) string {
	if n == 0 {
		return "0"
	}
	digits := []byte{}
	neg := n < 0
	if neg {
		n = -n
	}
	for n > 0 {
		digits = append([]byte{byte('0' + n%10)}, digits...)
		n /= 10
	}
	if neg {
		return "-" + string(digits)
	}
	return string(digits)
}
