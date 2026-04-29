// Package toolgate implements the K-16 PreToolUse class-split gate that
// blocks rain (EYES-class) from firing HANDS-class execute commands like
// `git commit` / `git push` / `gh pr create`. EYES-vs-HANDS class split
// per DISC v2 reserves execute actions for brian (HANDS); rain drafts +
// surfaces + greenflags but never fires.
//
// Closes the autocompact-induced class-split-violation failure class
// observed bcc-ad-manager session 2026-04-29 (Rain fired `gh issue create`
// at msg 6350 + `gh pr create` at msg 6358 directly when those are
// brian-owned execute actions).
//
// Pairs with K-12 (anchor-checksum drift detection in agentstate.go) and
// K-19 (force-push elevated-gate; consumes IsForcePushPattern from this
// package).
//
// Phase K K-16.
package toolgate

import "strings"

// IsHANDSExecutePattern returns true if the command line invokes an
// execute-class operation reserved for the HANDS=brian role per the
// EYES-vs-HANDS class split.
//
// Match is on token-1 + token-2 (the first two whitespace-delimited
// command tokens after quote-respecting tokenization). Substring matches
// inside echo / grep / cat / comments do NOT trigger because those
// commands' first-2-tokens are `echo` + something-else, not `git` +
// `push`.
//
// Rain refinement-2 (msg 6411): tokenize-first-2-tokens approach
// resists false-positives from `echo 'git push'`, `cat /tmp/git-push.log`,
// `grep "git push" file`, etc.
//
// MVP scope: Bash-pattern-matcher only. Edit/Write tool-call gating
// deferred to post-MVP K-16-extension if empirical observation shows
// rain drifts on file-edit-class actions.
//
// Chain-evasion (`&&`, `||`, `|`, `;` followed by execute pattern) is a
// known low-priority gap — we only inspect the FIRST command in the
// chain. If empirical drift surfaces via chains, K-21 ratchet candidate
// adds chain-aware matching.
func IsHANDSExecutePattern(command string) bool {
	tokens := tokenize(command)
	if len(tokens) < 2 {
		return false
	}
	// git: pair-match (git commit / git push / etc. are uniformly HANDS;
	// no read-class git subcommand is in the gate set).
	pair := tokens[0] + " " + tokens[1]
	if handsGitPairs[pair] {
		return true
	}
	// gh: triple-match (gh has both read subcommands like `gh pr view` and
	// write subcommands like `gh pr create`; pair-match on `gh pr` would
	// false-positive on the read variants).
	if len(tokens) < 3 {
		return false
	}
	triple := tokens[0] + " " + tokens[1] + " " + tokens[2]
	return handsGhTriples[triple]
}

// handsGitPairs enumerates the HANDS-only `git <subcommand>` pairs. All
// git subcommands listed here are write-class (no read-class git
// operation is in the gate set; `git status` / `git log` / `git diff` /
// `git show` / `git fetch` / `git branch` are EYES-allowed).
var handsGitPairs = map[string]bool{
	"git commit":      true,
	"git push":        true,
	"git reset":       true, // --hard / --soft / --mixed all HANDS-class write
	"git rebase":      true,
	"git merge":       true, // local merge — HANDS class even if not pushed
	"git revert":      true,
	"git cherry-pick": true,
}

// handsGhTriples enumerates the HANDS-only `gh <object> <verb>` triples.
// Read-class verbs (`view`, `list`, `diff`, `checkout`) are EYES-allowed
// and not included here. Adding new HANDS verbs (e.g., when gh adds new
// write subcommands) requires a new map entry.
var handsGhTriples = map[string]bool{
	// gh pr — write variants
	"gh pr create":  true,
	"gh pr edit":    true,
	"gh pr merge":   true,
	"gh pr close":   true,
	"gh pr reopen":  true,
	"gh pr review":  true, // --approve / --request-changes are HANDS-class
	"gh pr ready":   true,
	"gh pr comment": true,
	// gh issue — write variants
	"gh issue create":  true,
	"gh issue edit":    true,
	"gh issue close":   true,
	"gh issue reopen":  true,
	"gh issue comment": true,
	"gh issue delete":  true,
	// gh release — write variants
	"gh release create": true,
	"gh release edit":   true,
	"gh release delete": true,
	"gh release upload": true,
}

// IsForcePushPattern returns true if the command line invokes a
// force-push operation (`git push -f` / `--force` / `--force-with-lease`
// variants). Reserved for K-19 elevated-gate consumption: force-push has
// higher reversibility cost (rewrites history) so it requires both
// peer-greenflag AND user-explicit-verbatim per K-19 codification.
//
// K-16 itself doesn't act on this signal differently from regular
// HANDS-execute-pattern blocking; K-19 wires the elevated gate.
func IsForcePushPattern(command string) bool {
	tokens := tokenize(command)
	if len(tokens) < 2 || tokens[0] != "git" || tokens[1] != "push" {
		return false
	}
	for _, t := range tokens[2:] {
		if t == "-f" || t == "--force" || t == "--force-with-lease" {
			return true
		}
		if strings.HasPrefix(t, "--force-with-lease=") {
			return true
		}
	}
	return false
}

// tokenize splits a command string into whitespace-delimited tokens,
// respecting single-quoted and double-quoted spans (treated as single
// atomic tokens). Backtick command-substitution and $(...) are
// recognized as quote-class spans so substituted commands DO NOT leak
// to the top level.
//
// Bash sub-shell / pipe / && / || / ; chains: only the first command
// (up to the first chain operator) is tokenized — chain-evasion is a
// known gap deferred to K-21 candidate per Rain msg 6418.
//
// Not a full bash parser. Sufficient for the K-16 first-2-tokens
// HANDS-pattern check which is the MVP requirement.
func tokenize(command string) []string {
	command = strings.TrimSpace(command)
	if command == "" {
		return nil
	}

	// Truncate at first chain operator so we only inspect the FIRST
	// command in the chain. (Order matters: longer ops first.)
	for _, op := range []string{"&&", "||", "|", ";"} {
		if i := indexOutsideQuotes(command, op); i >= 0 {
			command = strings.TrimSpace(command[:i])
		}
	}

	var tokens []string
	var current strings.Builder
	i := 0
	for i < len(command) {
		c := command[i]
		switch {
		case c == ' ' || c == '\t':
			if current.Len() > 0 {
				tokens = append(tokens, current.String())
				current.Reset()
			}
			i++
		case c == '\'':
			// Single-quoted span: literal up to next '
			end := strings.IndexByte(command[i+1:], '\'')
			if end < 0 {
				current.WriteString(command[i:])
				i = len(command)
			} else {
				current.WriteString(command[i : i+1+end+1])
				i += 1 + end + 1
			}
		case c == '"':
			// Double-quoted span: literal up to next " (no $ expansion concern; we don't evaluate)
			end := strings.IndexByte(command[i+1:], '"')
			if end < 0 {
				current.WriteString(command[i:])
				i = len(command)
			} else {
				current.WriteString(command[i : i+1+end+1])
				i += 1 + end + 1
			}
		case c == '`':
			// Backtick command-substitution: treat as atomic span
			end := strings.IndexByte(command[i+1:], '`')
			if end < 0 {
				current.WriteString(command[i:])
				i = len(command)
			} else {
				current.WriteString(command[i : i+1+end+1])
				i += 1 + end + 1
			}
		case c == '$' && i+1 < len(command) && command[i+1] == '(':
			// $(...) command-substitution: scan to matching close-paren
			depth := 1
			j := i + 2
			for j < len(command) && depth > 0 {
				if command[j] == '(' {
					depth++
				} else if command[j] == ')' {
					depth--
				}
				j++
			}
			current.WriteString(command[i:j])
			i = j
		default:
			current.WriteByte(c)
			i++
		}
	}
	if current.Len() > 0 {
		tokens = append(tokens, current.String())
	}
	return tokens
}

// indexOutsideQuotes finds the first index of `needle` in `s` that is
// not inside a single-quoted, double-quoted, or backtick-quoted span.
// Returns -1 if no such occurrence.
func indexOutsideQuotes(s, needle string) int {
	i := 0
	for i < len(s) {
		c := s[i]
		switch c {
		case '\'':
			end := strings.IndexByte(s[i+1:], '\'')
			if end < 0 {
				return -1
			}
			i = i + 1 + end + 1
		case '"':
			end := strings.IndexByte(s[i+1:], '"')
			if end < 0 {
				return -1
			}
			i = i + 1 + end + 1
		case '`':
			end := strings.IndexByte(s[i+1:], '`')
			if end < 0 {
				return -1
			}
			i = i + 1 + end + 1
		default:
			if i+len(needle) <= len(s) && s[i:i+len(needle)] == needle {
				return i
			}
			i++
		}
	}
	return -1
}
