// Package toolgate — r49.go: R49 PRE-SEAL-MECHANICAL-AUDIT hook
// per phase-t.md v5 T-2.5.
//
// Wires the citeanchor + cl validators into the PreToolUse hook chain.
// On Write/Edit operations targeting scope-lock-doc class artifacts
// (~/.bot-hq/projects/<project>/phase/*.md, post-Z-1), runs cite-anchor
// validation on the new content + reports invalid anchors as warnings
// (default) or blocks (strict mode).
//
// Catches G3-class failures empirically: Phase S 13-miss-failure case
// where bilateral seal failed to catch architectural-fork + 5 deliverables
// + 3 internal contradictions (docs/arcs/phase-s.md:13).
//
// Mode policy:
//   - WARN (default): log warning + emma alert; allow tool call
//   - BLOCK (strict): log + emit block-reason; reject tool call
//
// Mode controlled by env-var BOT_HQ_R49_MODE: "warn" (default) or "block".
package toolgate

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/citeanchor"
)

const (
	// R49ModeWarn allows tool calls but emits warnings (default mode).
	R49ModeWarn = "warn"

	// R49ModeBlock blocks tool calls when invalid anchors detected (strict mode).
	R49ModeBlock = "block"
)

// R49Verdict is the result of a R49 pre-seal-audit check.
type R49Verdict struct {
	ShouldBlock bool
	Mode        string
	Result      citeanchor.ValidationResult
	Reason      string // human-readable summary for stderr
}

// R49PreSealAudit runs the R49 cite-anchor validation on a file's new
// content. Returns R49Verdict indicating whether the tool call should
// be blocked + a human-readable summary.
//
// Scope: only fires for paths matching scope-lock-doc class (e.g.
// ~/.bot-hq/projects/<project>/phase/*.md, post-Z-1; legacy
// ~/.bot-hq/phase/*.md still recognized for transition-window safety).
// Non-scope-lock paths return ShouldBlock=false with empty result.
//
// Mode: env-var BOT_HQ_R49_MODE controls block-vs-warn behavior.
// Default is "warn".
func R49PreSealAudit(ctx context.Context, filePath string, newContent []byte) R49Verdict {
	if !IsScopeLockDoc(filePath) {
		return R49Verdict{}
	}

	mode := os.Getenv("BOT_HQ_R49_MODE")
	if mode == "" {
		mode = R49ModeWarn
	}

	res := citeanchor.ValidateString(ctx, string(newContent), filePath, nil)

	verdict := R49Verdict{
		Mode:   mode,
		Result: res,
	}

	if res.Invalid == 0 {
		verdict.Reason = fmt.Sprintf("R49 audit PASS: %d anchors / %d valid / %d skipped", len(res.Anchors), res.Valid, res.Skipped)
		return verdict
	}

	verdict.Reason = formatR49FailReason(filePath, res)

	if mode == R49ModeBlock {
		verdict.ShouldBlock = true
	}

	return verdict
}

// IsScopeLockDoc reports whether a file path matches the scope-lock-doc
// class convention. Post-Z-1: any *.md under ~/.bot-hq/projects/<p>/phase/
// qualifies (project-scoped). Legacy ~/.bot-hq/phase/*.md still recognized
// during transition window; post-soak (~7d) the legacy substring check
// can be dropped.
func IsScopeLockDoc(path string) bool {
	abs, err := filepath.Abs(path)
	if err != nil {
		abs = path
	}
	if !strings.HasSuffix(abs, ".md") {
		return false
	}
	// Z-1 transition: cover BOTH old top-level and new project-scoped paths.
	// Post-soak simplification: drop the legacy "/.bot-hq/phase/" check.
	if strings.Contains(abs, "/.bot-hq/phase/") {
		return true
	}
	if strings.Contains(abs, "/.bot-hq/projects/") && strings.Contains(abs, "/phase/") {
		return true
	}
	return false
}

func formatR49FailReason(filePath string, res citeanchor.ValidationResult) string {
	var sb strings.Builder
	fmt.Fprintf(&sb, "R49 PRE-SEAL-AUDIT findings on %s:\n", filePath)
	fmt.Fprintf(&sb, "  Total anchors: %d / Valid: %d / Invalid: %d / Skipped: %d\n", len(res.Anchors), res.Valid, res.Invalid, res.Skipped)
	fmt.Fprintln(&sb, "  First invalid anchors:")

	count := 0
	for _, f := range res.Findings {
		if f.Status != "invalid" {
			continue
		}
		count++
		if count > 10 {
			fmt.Fprintf(&sb, "  ... (%d more invalid; truncated)\n", res.Invalid-10)
			break
		}
		fmt.Fprintf(&sb, "    - [%s] %q: %s\n", f.Anchor.Class, f.Anchor.Raw, f.Message)
	}
	fmt.Fprintln(&sb, "Recovery: fix invalid anchors OR set BOT_HQ_R49_MODE=warn to bypass mechanical-block.")
	return sb.String()
}
