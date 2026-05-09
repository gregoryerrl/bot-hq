package citeanchor

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// stubMsgChecker is a controllable test double for MsgIDChecker.
type stubMsgChecker struct {
	exists map[int64]bool
	err    error
}

func (s *stubMsgChecker) MessageExists(_ context.Context, id int64) (bool, error) {
	if s.err != nil {
		return false, s.err
	}
	return s.exists[id], nil
}

func TestExtractAnchors_msgIDs(t *testing.T) {
	content := `Per Brian msg 17094 + Rain msg 17128 + msg-17146 user pre-delegation.
Also see msg17150 + (msg 17151).`
	got := ExtractAnchors(content, "<inline>")

	wantValues := []string{"17094", "17128", "17146", "17150", "17151"}
	if len(got) != len(wantValues) {
		t.Fatalf("anchor count = %d, want %d; got: %v", len(got), len(wantValues), got)
	}
	for i, w := range wantValues {
		if got[i].Value != w {
			t.Errorf("anchor[%d] value = %q, want %q", i, got[i].Value, w)
		}
		if got[i].Class != "msg-id" {
			t.Errorf("anchor[%d] class = %q, want msg-id", i, got[i].Class)
		}
	}
}

func TestExtractAnchors_filePaths(t *testing.T) {
	content := `Per /Users/gregoryerrl/.bot-hq/phase/phase-t.md + ~/Projects/bot-hq/internal/hub/db.go.`
	got := ExtractAnchors(content, "<inline>")

	if len(got) != 2 {
		t.Fatalf("anchor count = %d, want 2; got: %v", len(got), got)
	}
	if got[0].Value != "/Users/gregoryerrl/.bot-hq/phase/phase-t.md" {
		t.Errorf("anchor[0] value = %q", got[0].Value)
	}
	if got[1].Value != "~/Projects/bot-hq/internal/hub/db.go" {
		t.Errorf("anchor[1] value = %q", got[1].Value)
	}
}

func TestExtractAnchors_lineAnchors(t *testing.T) {
	content := `Per docs/arcs/phase-s.md:13 + /Users/x/file.go:50-60.`
	got := ExtractAnchors(content, "<inline>")

	if len(got) != 1 {
		t.Fatalf("expected 1 anchor (only absolute paths matched); got: %v", got)
	}
	a := got[0]
	if a.Class != "line-anchor" {
		t.Errorf("class = %q, want line-anchor", a.Class)
	}
	if a.LineHint != 50 {
		t.Errorf("LineHint = %d, want 50", a.LineHint)
	}
	if a.LineSpan != 60 {
		t.Errorf("LineSpan = %d, want 60", a.LineSpan)
	}
}

func TestExtractAnchors_excludesURLs(t *testing.T) {
	content := `See https://api.deepseek.com/anthropic for the Anthropic-compatible endpoint.
Also https://github.com/foo/bar/blob/main/foo.go for the source.`
	got := ExtractAnchors(content, "<inline>")
	for _, a := range got {
		if strings.Contains(a.Value, "deepseek.com") || strings.Contains(a.Value, "github.com") {
			t.Errorf("URL fragment incorrectly extracted as file-path: %q", a.Value)
		}
	}
}

func TestValidate_msgID_existing(t *testing.T) {
	stub := &stubMsgChecker{exists: map[int64]bool{17094: true}}
	a := CiteAnchor{Class: "msg-id", Value: "17094", Raw: "msg 17094"}
	f := Validate(context.Background(), a, stub)
	if f.Status != "valid" {
		t.Errorf("status = %q, want valid; msg: %s", f.Status, f.Message)
	}
}

func TestValidate_msgID_missing(t *testing.T) {
	stub := &stubMsgChecker{exists: map[int64]bool{17094: true}} // 99999 not in map
	a := CiteAnchor{Class: "msg-id", Value: "99999", Raw: "msg 99999"}
	f := Validate(context.Background(), a, stub)
	if f.Status != "invalid" {
		t.Errorf("status = %q, want invalid; msg: %s", f.Status, f.Message)
	}
}

func TestValidate_msgID_lookupError_returnsWarning(t *testing.T) {
	stub := &stubMsgChecker{err: errors.New("db unreachable")}
	a := CiteAnchor{Class: "msg-id", Value: "17094"}
	f := Validate(context.Background(), a, stub)
	if f.Status != "warning" {
		t.Errorf("status = %q, want warning", f.Status)
	}
}

func TestValidate_msgID_nilChecker_skipped(t *testing.T) {
	a := CiteAnchor{Class: "msg-id", Value: "17094"}
	f := Validate(context.Background(), a, nil)
	if f.Status != "skipped" {
		t.Errorf("status = %q, want skipped", f.Status)
	}
}

func TestValidate_filePath_existing(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "test.txt")
	if err := os.WriteFile(path, []byte("hello"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	a := CiteAnchor{Class: "file-path", Value: path, Raw: path}
	f := Validate(context.Background(), a, nil)
	if f.Status != "valid" {
		t.Errorf("status = %q, want valid; msg: %s", f.Status, f.Message)
	}
}

func TestValidate_filePath_missing(t *testing.T) {
	a := CiteAnchor{Class: "file-path", Value: "/nonexistent/path/xyz.md"}
	f := Validate(context.Background(), a, nil)
	if f.Status != "invalid" {
		t.Errorf("status = %q, want invalid", f.Status)
	}
}

func TestValidate_lineAnchor_inBounds(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "test.go")
	if err := os.WriteFile(path, []byte("line1\nline2\nline3\nline4\nline5\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	a := CiteAnchor{Class: "line-anchor", Value: path, LineHint: 3}
	f := Validate(context.Background(), a, nil)
	if f.Status != "valid" {
		t.Errorf("status = %q, want valid; msg: %s", f.Status, f.Message)
	}
}

func TestValidate_lineAnchor_outOfBounds(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "test.go")
	if err := os.WriteFile(path, []byte("line1\nline2\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	a := CiteAnchor{Class: "line-anchor", Value: path, LineHint: 100}
	f := Validate(context.Background(), a, nil)
	if f.Status != "invalid" {
		t.Errorf("status = %q, want invalid", f.Status)
	}
}

func TestValidate_lineSpan_outOfBounds(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "test.go")
	if err := os.WriteFile(path, []byte("line1\nline2\nline3\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	a := CiteAnchor{Class: "line-anchor", Value: path, LineHint: 1, LineSpan: 999}
	f := Validate(context.Background(), a, nil)
	if f.Status != "invalid" {
		t.Errorf("status = %q, want invalid", f.Status)
	}
}

func TestValidateString_aggregates(t *testing.T) {
	dir := t.TempDir()
	existPath := filepath.Join(dir, "exists.md")
	if err := os.WriteFile(existPath, []byte("line1\nline2\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	stub := &stubMsgChecker{exists: map[int64]bool{17094: true, 17128: true}}

	// Use /tmp prefix (regex matches /tmp/...) for the missing-file test
	content := "Cites: msg 17094, msg 99999, " + existPath + ", /tmp/nonexistent-xyz-test-file.md, msg 17128"

	res := ValidateString(context.Background(), content, "<inline>", stub)

	if res.Valid != 3 {
		t.Errorf("valid = %d, want 3 (msg 17094, %s, msg 17128)", res.Valid, existPath)
	}
	if res.Invalid != 2 {
		t.Errorf("invalid = %d, want 2 (msg 99999, /tmp/nonexistent-xyz-test-file.md)", res.Invalid)
	}
}

func TestValidateFile_endToEnd(t *testing.T) {
	dir := t.TempDir()
	docPath := filepath.Join(dir, "scope-lock.md")

	// Self-referential test: doc contains a cite to itself + a cite to a missing file
	content := "Cites: " + docPath + " (self-ref) + /tmp/nonexistent-missing-xyz-abc.md"
	if err := os.WriteFile(docPath, []byte(content), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}

	res, err := ValidateFile(context.Background(), docPath, nil)
	if err != nil {
		t.Fatalf("ValidateFile: %v", err)
	}
	if res.Valid < 1 {
		t.Errorf("valid = %d, want >= 1 (self-ref should resolve)", res.Valid)
	}
	if res.Invalid < 1 {
		t.Errorf("invalid = %d, want >= 1 (missing file)", res.Invalid)
	}
}
