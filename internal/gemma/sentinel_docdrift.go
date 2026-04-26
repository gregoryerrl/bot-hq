package gemma

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
)

// docDriftSentinelName is the ledger filename prefix for the H-23 doc-drift
// sentinel. Periodic-scan class (distinct from the hub-message-reactive
// pattern class in sentinel.go) — no regex appears in dryRunPatterns;
// instead the entire sentinel writes to the same dry-run ledger by
// convention. After Rain reviews ≤5% false-positive rate, the sentinel
// can be promoted to live MsgUpdate-to-Rain by editing
// docDriftDryRunActive.
const docDriftSentinelName = "docdrift"

// docDriftDryRunActive controls whether observations write to the dry-run
// ledger (true) or send via hub_send to Rain (false). Slice 2 ships in
// dry-run; flip after tuning-gate green.
var docDriftDryRunActive = true

// arcStatusOpenRE matches an arc.md "Status: open" line. The arc.md
// closure-discipline convention puts Status as the file's third line
// (e.g. "Status: open  | Branch: ..."); detection is line-anchored.
var arcStatusOpenRE = regexp.MustCompile(`(?m)^Status:\s*open\b`)

// arcRefRE extracts backtick-fenced tokens that LOOK like branch or SHA
// references. Tokens may be SHAs (`b06961c`), branches (`brian/phase-h-
// slice-1`), or branch-with-sha (`brian/phase-h-slice-1@b06961c`). We
// pull the literal token and let git classify — non-refs return no
// match in `git show-ref` / `git merge-base` and are silently skipped.
var arcRefRE = regexp.MustCompile("`([\\w./@-]+)`")

// shaShapeRE detects a hex token of length 7-40 (typical short or full
// git SHAs). Used to classify extracted refs as SHA-vs-branch.
var shaShapeRE = regexp.MustCompile(`^[a-f0-9]{7,40}$`)

// DocDriftObservation captures one drift finding from a periodic scan.
type DocDriftObservation struct {
	ArcPath   string // relative path under arcsDir
	Kind      string // "merged-branch" | "ancestor-sha"
	Reference string // the branch name or sha
}

// ScanArcsForDocDrift walks `arcsDir` for *.md files, filters to those
// whose first lines contain `Status: open`, extracts backtick-fenced
// references, and returns observations for refs whose tip (or self,
// for SHAs) is already an ancestor of `origin/main` in `repoDir`.
//
// Periodic-scan class: caller invokes on a timer (e.g. every 30 min).
// Pure I/O — no hub interaction; emit to ledger / Rain is the caller's
// responsibility (use EmitDocDriftObservations).
//
// SHAs are extracted from the same backtick-fenced token regex and then
// classified via shaShapeRE. Branch refs ending in `@<sha>` are split:
// the branch part feeds the merged-branch check, the sha part feeds the
// ancestor-sha check.
func ScanArcsForDocDrift(arcsDir, repoDir string) ([]DocDriftObservation, error) {
	files, err := filepath.Glob(filepath.Join(arcsDir, "*.md"))
	if err != nil {
		return nil, fmt.Errorf("glob arcs: %w", err)
	}
	// Scaling note: every backticked token shells out to git for
	// classification. Acceptable at v1 scale (~50 backticks per arc, ~2
	// open arcs). If arcs grow past ~500 backticks total, pre-filter by
	// shape (heuristic prefix or hex shape) before shelling out.
	var observations []DocDriftObservation
	seen := make(map[string]struct{})
	for _, path := range files {
		data, err := os.ReadFile(path)
		if err != nil {
			continue // best-effort scan
		}
		if !arcStatusOpenRE.Match(data) {
			continue // closed (or no Status line) — out of scope per H-23
		}
		text := string(data)
		for _, m := range arcRefRE.FindAllStringSubmatch(text, -1) {
			token := m[1]
			// Split branch@sha into both pieces.
			refs := []string{token}
			if at := strings.Index(token, "@"); at > 0 {
				refs = []string{token[:at], token[at+1:]}
			}
			for _, ref := range refs {
				key := path + "|" + ref
				if _, dup := seen[key]; dup {
					continue
				}
				if shaShapeRE.MatchString(ref) {
					if isAncestorOfOriginMain(repoDir, ref) {
						observations = append(observations, DocDriftObservation{
							ArcPath: path, Kind: "ancestor-sha", Reference: ref,
						})
						seen[key] = struct{}{}
					}
				} else if isBranchMergedToMain(repoDir, ref) {
					observations = append(observations, DocDriftObservation{
						ArcPath: path, Kind: "merged-branch", Reference: ref,
					})
					seen[key] = struct{}{}
				}
			}
		}
	}
	return observations, nil
}

// isAncestorOfOriginMain reports whether `sha` is an ancestor of
// `origin/main` in repoDir. Returns false on any git error (best-effort).
func isAncestorOfOriginMain(repoDir, sha string) bool {
	cmd := exec.Command("git", "-C", repoDir, "merge-base", "--is-ancestor", sha, "origin/main")
	return cmd.Run() == nil
}

// isBranchMergedToMain reports whether `branch` exists as `origin/<branch>`
// in repoDir AND its tip is an ancestor of `origin/main`. Returns false
// for non-existent or unmerged branches (best-effort).
//
// Self-reference filter: `main` and `master` always satisfy "tip is
// ancestor of main" (a tip is its own ancestor). Treat those as not-
// drift to avoid noise on every arc that mentions the trunk by name.
func isBranchMergedToMain(repoDir, branch string) bool {
	if branch == "main" || branch == "master" {
		return false
	}
	out, err := exec.Command("git", "-C", repoDir, "rev-parse", "origin/"+branch).Output()
	if err != nil {
		return false // ref doesn't exist
	}
	tip := strings.TrimSpace(string(out))
	return isAncestorOfOriginMain(repoDir, tip)
}

// EmitDocDriftObservations writes each observation to the appropriate
// destination. During the dry-run period (docDriftDryRunActive == true),
// observations append to the docdrift ledger. Post-tuning-gate, this
// would emit hub MsgUpdate to Rain — wired in slice 3 lifecycle work
// (deferred per slice 2 design out-of-scope: "multi-sentinel coordinator").
func EmitDocDriftObservations(observations []DocDriftObservation) {
	if !docDriftDryRunActive {
		// TODO(slice 3): replace with hub_send MsgUpdate to Rain via the
		// universal sentinel coordinator (master design "Out of scope" item:
		// multi-sentinel coordinator). Until that wiring lands, flipping
		// docDriftDryRunActive to false silently DROPS observations — do
		// NOT flip the flag without slice 3's coordinator in place.
		return
	}
	for _, o := range observations {
		AppendToDryRunLedger(docDriftSentinelName, fmt.Sprintf("arc=%s | kind=%s | ref=%s", relPath(o.ArcPath), o.Kind, o.Reference))
	}
}

// relPath best-effort shortens an absolute path to a path relative to
// the current working directory. Used for ledger readability — falls
// back to the absolute path on error.
func relPath(abs string) string {
	cwd, err := os.Getwd()
	if err != nil {
		return abs
	}
	r, err := filepath.Rel(cwd, abs)
	if err != nil {
		return abs
	}
	return r
}
