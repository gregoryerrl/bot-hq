package sessions

// Tests for secrets-scan-on-manifest-author (P-6 / phase-n.md:294 +
// OQ-6). Pattern set is conservative — high-confidence detectors only.
// Tests pin: each provider pattern matches a synthetic-but-realistic
// example; clean-input has zero findings; redaction substitutes
// placeholder; WriteManifest integration logs + (in strict mode)
// errors on hit.

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// scanCases enumerates the canonical synthetic samples for each
// pattern. Samples are clearly-fake-but-correctly-structured so the
// regex matches without tripping GitHub Push Protection scanners on
// the test file itself. "FAKE-TEST-NOT-REAL" markers in the high-
// entropy portion suppress known-secret heuristics while preserving
// regex-class structure (prefix + length).
var scanCases = []struct {
	name    string
	input   string
	pattern string
}{
	{"anthropic-key", "key = sk-ant-FAKE_TEST_NOT_REAL_SAMPLE_0000_for_unit_tests", "anthropic-api-key"},
	{"openai-key", "OPENAI_API_KEY=sk-FAKETESTNOTREALSAMPLE0000000000FORUNITTESTS00", "openai-api-key"},
	{"github-pat", "token: ghp_FAKETESTNOTREALSAMPLE000000000000000000", "github-personal-token"},
	{"github-oauth", "token: gho_FAKETESTNOTREALSAMPLE000000000000000000", "github-personal-token"},
	{"aws-access-key", "aws_id = AKIAFAKETESTSAMPLE00", "aws-access-key-id"},
	{"private-key-block", "-----BEGIN RSA PRIVATE KEY-----\n[FAKE TEST CONTENT NOT A REAL KEY]", "private-key-block"},
	{"openssh-private", "-----BEGIN OPENSSH PRIVATE KEY-----", "private-key-block"},
	{"slack-bot-token", "slack: xoxb-FAKE-TEST-NOT-REAL-SAMPLE-FOR-UNIT-TESTS-ONLY", "slack-token"},
	{"password-assignment", `password: "FAKE_TEST_PWD_NOT_REAL"`, "generic-password-assignment"},
	{"token-assignment", `api_key: "FAKETESTSAMPLE_NOT_A_REAL_TOKEN"`, "generic-token-assignment"},
}

// TestScanForSecrets_DetectsPatterns is the load-bearing case: each
// canonical sample should produce ≥1 finding with the expected pattern
// name.
func TestScanForSecrets_DetectsPatterns(t *testing.T) {
	for _, tc := range scanCases {
		t.Run(tc.name, func(t *testing.T) {
			findings := ScanForSecrets(tc.input)
			if len(findings) == 0 {
				t.Errorf("expected ≥1 finding for %q, got 0", tc.input)
				return
			}
			matched := false
			for _, f := range findings {
				if f.Pattern == tc.pattern {
					matched = true
					if f.Line < 1 {
						t.Errorf("finding line should be 1-indexed; got %d", f.Line)
					}
					if f.Snippet == "" {
						t.Errorf("finding snippet should be non-empty")
					}
				}
			}
			if !matched {
				t.Errorf("findings %v missing expected pattern %q", findings, tc.pattern)
			}
		})
	}
}

// TestScanForSecrets_CleanInput verifies normal manifest content
// without secret patterns produces zero findings.
func TestScanForSecrets_CleanInput(t *testing.T) {
	clean := `---
id: 2026-05-07-bot-hq
project: bot-hq
agents: brian, rain
---

# Phase P drain progress

P-1 SSE landed. P-2 CodeMirror landed. Working on P-6 secrets scan.
No credentials, tokens, or keys here — just a normal manifest body.
`
	findings := ScanForSecrets(clean)
	if len(findings) != 0 {
		t.Errorf("clean input should produce 0 findings; got %v", findings)
	}
}

// TestScanForSecrets_EmptyInput returns nil (not error) for empty.
func TestScanForSecrets_EmptyInput(t *testing.T) {
	if got := ScanForSecrets(""); got != nil {
		t.Errorf("empty input should produce nil; got %v", got)
	}
}

// TestScanForSecrets_LineNumberAccurate confirms 1-indexed line
// numbers correctly identify the offending line.
func TestScanForSecrets_LineNumberAccurate(t *testing.T) {
	input := "line one\nline two\nsecret: ghp_FAKETESTNOTREALSAMPLE000000000000000000\nline four"
	findings := ScanForSecrets(input)
	if len(findings) != 1 {
		t.Fatalf("expected 1 finding, got %d", len(findings))
	}
	if findings[0].Line != 3 {
		t.Errorf("finding line = %d, want 3", findings[0].Line)
	}
}

// TestScanForSecrets_SnippetTruncated verifies the snippet caps at
// snippetMaxLen with ellipsis so log files don't echo full secrets.
func TestScanForSecrets_SnippetTruncated(t *testing.T) {
	long := "ghp_FAKETEST_" + strings.Repeat("X", 100)
	findings := ScanForSecrets(long)
	if len(findings) == 0 {
		t.Fatal("expected ≥1 finding")
	}
	if !strings.Contains(findings[0].Snippet, "…") {
		t.Errorf("long match snippet should be truncated with ellipsis; got %q", findings[0].Snippet)
	}
	if len(findings[0].Snippet) > snippetMaxLen+3 {
		t.Errorf("snippet too long: %d chars", len(findings[0].Snippet))
	}
}

// TestRedactSecrets_ReplacesAll verifies redaction substitutes the
// placeholder for each match + reports the findings.
func TestRedactSecrets_ReplacesAll(t *testing.T) {
	input := "before ghp_FAKETESTNOTREALSAMPLE000000000000000000 after"
	out, findings := RedactSecrets(input)
	if !strings.Contains(out, scanRedactPlaceholder) {
		t.Errorf("output missing placeholder: %s", out)
	}
	if strings.Contains(out, "FAKETESTNOT") {
		t.Errorf("output still contains secret value: %s", out)
	}
	if len(findings) != 1 {
		t.Errorf("expected 1 finding, got %d", len(findings))
	}
}

// TestLogSecretFindings_AppendsToLog verifies the log-append helper
// writes findings + creates dir if missing.
func TestLogSecretFindings_AppendsToLog(t *testing.T) {
	tmp := t.TempDir()
	logPath := filepath.Join(tmp, "subdir", "secrets-log.md")
	t.Setenv("BOT_HQ_SECRETS_LOG_PATH", logPath)

	LogSecretFindings("2026-05-07-test", []SecretFinding{
		{Pattern: "github-pat", Line: 3, Snippet: "ghp_xxx"},
	})

	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("read log: %v", err)
	}
	body := string(data)
	if !strings.Contains(body, "2026-05-07-test") {
		t.Errorf("log missing manifest id: %s", body)
	}
	if !strings.Contains(body, "github-pat") {
		t.Errorf("log missing pattern name: %s", body)
	}
	if !strings.Contains(body, "L3") {
		t.Errorf("log missing line number: %s", body)
	}
}

// TestLogSecretFindings_NoOpOnEmpty verifies passing an empty findings
// slice does not create the log file (no spurious empty entries).
func TestLogSecretFindings_NoOpOnEmpty(t *testing.T) {
	tmp := t.TempDir()
	logPath := filepath.Join(tmp, "secrets-log.md")
	t.Setenv("BOT_HQ_SECRETS_LOG_PATH", logPath)

	LogSecretFindings("2026-05-07-test", nil)

	if _, err := os.Stat(logPath); !os.IsNotExist(err) {
		t.Errorf("log file should not exist for empty findings; stat err=%v", err)
	}
}

// TestWriteManifest_CleanInput_NoLog verifies the integration: a clean
// manifest writes successfully + does NOT produce a secrets-log entry.
func TestWriteManifest_CleanInput_NoLog(t *testing.T) {
	setSessionsDir(t)
	tmp := t.TempDir()
	logPath := filepath.Join(tmp, "secrets-log.md")
	t.Setenv("BOT_HQ_SECRETS_LOG_PATH", logPath)

	m := Manifest{
		ID:      "2026-05-07-clean",
		Project: "bot-hq",
		Body:    "Just a normal manifest body. No secrets here.\n",
	}
	if err := WriteManifest(m); err != nil {
		t.Fatalf("write manifest: %v", err)
	}
	if _, err := os.Stat(logPath); !os.IsNotExist(err) {
		t.Errorf("clean manifest should not produce secrets-log entry")
	}
}

// TestWriteManifest_SecretInBody_Logs verifies the integration: a
// manifest body containing a secret pattern produces a secrets-log
// entry but the manifest write still succeeds (default policy).
func TestWriteManifest_SecretInBody_Logs(t *testing.T) {
	setSessionsDir(t)
	tmp := t.TempDir()
	logPath := filepath.Join(tmp, "secrets-log.md")
	t.Setenv("BOT_HQ_SECRETS_LOG_PATH", logPath)

	m := Manifest{
		ID:      "2026-05-07-leak",
		Project: "bot-hq",
		Body:    "oops: ghp_FAKETESTNOTREALSAMPLE000000000000000000\n",
	}
	if err := WriteManifest(m); err != nil {
		t.Fatalf("write manifest (default policy should not error): %v", err)
	}
	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("read secrets-log: %v", err)
	}
	if !strings.Contains(string(data), "2026-05-07-leak") {
		t.Errorf("secrets-log missing manifest id: %s", string(data))
	}
	if !strings.Contains(string(data), "github-personal-token") {
		t.Errorf("secrets-log missing pattern: %s", string(data))
	}
}

// TestWriteManifest_StrictMode_ErrorsOnSecret verifies BOT_HQ_SECRETS_
// STRICT=1 returns an error when secrets are detected (caller decides
// retry/redact).
func TestWriteManifest_StrictMode_ErrorsOnSecret(t *testing.T) {
	setSessionsDir(t)
	tmp := t.TempDir()
	logPath := filepath.Join(tmp, "secrets-log.md")
	t.Setenv("BOT_HQ_SECRETS_LOG_PATH", logPath)
	t.Setenv("BOT_HQ_SECRETS_STRICT", "1")

	m := Manifest{
		ID:      "2026-05-07-strict",
		Project: "bot-hq",
		Body:    "leak: AKIAFAKETESTSAMPLE00\n",
	}
	err := WriteManifest(m)
	if err == nil {
		t.Fatal("expected error in strict mode with secret in body")
	}
	if !strings.Contains(err.Error(), "secret-pattern hit") {
		t.Errorf("error should mention secret-pattern hit; got %v", err)
	}
}
