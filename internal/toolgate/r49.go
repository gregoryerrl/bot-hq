// Package toolgate — r49.go: R49 PRE-SEAL-MECHANICAL-AUDIT hook
// per phase-t.md v5 T-2.5.
//
// Wires the citeanchor + cl validators into the PreToolUse hook chain.
// On Write/Edit operations targeting scope-lock-doc class artifacts
// (~/.bot-hq/phase/*.md), runs cite-anchor validation on the new content
// + reports invalid anchors as warnings (default) or blocks (strict mode).
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
// ~/.bot-hq/phase/*.md). Non-scope-lock paths return ShouldBlock=false
// with empty result.
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
// class convention. Currently: any *.md under ~/.bot-hq/phase/ qualifies.
func IsScopeLockDoc(path string) bool {
	abs, err := filepath.Abs(path)
	if err != nil {
		abs = path
	}
	home, _ := os.UserHomeDir()
	if home == "" {
		// Fall back to substring check
		return strings.Contains(abs, "/.bot-hq/phase/") && strings.HasSuffix(abs, ".md")
	}
	scopeLockDir := filepath.Join(home, ".bot-hq", "phase")
	rel, err := filepath.Rel(scopeLockDir, abs)
	if err != nil {
		return false
	}
	if strings.HasPrefix(rel, "..") {
		return false
	}
	return strings.HasSuffix(abs, ".md")
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
