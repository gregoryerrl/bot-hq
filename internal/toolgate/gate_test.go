package toolgate

import "testing"

// TestIsHANDSExecutePattern_Positive locks the HANDS-class command pairs
// the gate must reject for rain. Phase K K-16.
func TestIsHANDSExecutePattern_Positive(t *testing.T) {
	cases := []string{
		`git commit -m "msg"`,
		`git push origin main`,
		`git push -u origin branch-name`,
		`git reset --hard HEAD`,
		`git rebase -i HEAD~3`,
		`git merge feature-branch`,
		`git revert HEAD`,
		`git cherry-pick abc123`,
		`gh pr create --title "X" --body "Y"`,
		`gh pr merge 366 --squash`,
		`gh pr edit 366`,
		`gh issue create --title "X"`,
		`gh issue close 365`,
		`gh release create v1.0`,
	}
	for _, cmd := range cases {
		if !IsHANDSExecutePattern(cmd) {
			t.Errorf("IsHANDSExecutePattern(%q) = false, want true", cmd)
		}
	}
}

// TestIsHANDSExecutePattern_NegativeFalsePositive locks the false-positive
// resistance per Rain refinement-2 (msg 6411): substring matches inside
// echo / cat / grep / quoted-strings / comments must NOT trigger the gate.
func TestIsHANDSExecutePattern_NegativeFalsePositive(t *testing.T) {
	cases := []string{
		`echo 'git push'`,
		`echo "we'll git push later"`,
		`cat /tmp/git-push.log`,
		`grep "git push" file.txt`,
		`# git push`,
		`ls git-push-script.sh`,
	}
	for _, cmd := range cases {
		if IsHANDSExecutePattern(cmd) {
			t.Errorf("IsHANDSExecutePattern(%q) = true, want false (substring inside non-execute command)", cmd)
		}
	}
}

// TestIsHANDSExecutePattern_NegativeAllowed locks the read-only / safe
// commands that must be allowed for rain (EYES class can read git/gh).
func TestIsHANDSExecutePattern_NegativeAllowed(t *testing.T) {
	cases := []string{
		`ls -la`,
		`git status`,
		`git status -sb`,
		`git log --oneline -10`,
		`git diff`,
		`git diff --stat`,
		`git show abc123`,
		`git fetch origin`,
		`git branch -a`,
		`gh pr view 366`,
		`gh pr list --state open`,
		`gh issue view 365`,
		`gh issue list`,
		`gh repo view`,
	}
	for _, cmd := range cases {
		if IsHANDSExecutePattern(cmd) {
			t.Errorf("IsHANDSExecutePattern(%q) = true, want false (read-only operation)", cmd)
		}
	}
}

// TestIsForcePushPattern_Positive locks the force-push variants for K-19
// elevated-gate consumption.
func TestIsForcePushPattern_Positive(t *testing.T) {
	cases := []string{
		`git push -f`,
		`git push --force`,
		`git push --force-with-lease`,
		`git push --force-with-lease=origin/main`,
		`git push origin main -f`,
		`git push origin main --force`,
	}
	for _, cmd := range cases {
		if !IsForcePushPattern(cmd) {
			t.Errorf("IsForcePushPattern(%q) = false, want true", cmd)
		}
	}
}

// TestIsForcePushPattern_Negative locks regular push (non-force) as
// not-force-push.
func TestIsForcePushPattern_Negative(t *testing.T) {
	cases := []string{
		`git push origin main`,
		`git push -u origin branch`,
		`git push`,
		`echo "git push --force"`,
		`git status`,
		`git commit -m "msg"`,
	}
	for _, cmd := range cases {
		if IsForcePushPattern(cmd) {
			t.Errorf("IsForcePushPattern(%q) = true, want false", cmd)
		}
	}
}

// TestTokenize_RespectsQuotes locks the quote-respect invariant: quoted
// spans tokenize as single atomic tokens so substring matches inside
// don't leak to the top level.
func TestTokenize_RespectsQuotes(t *testing.T) {
	cases := map[string][]string{
		`echo 'git push'`:           {`echo`, `'git push'`},
		`echo "git push"`:           {`echo`, `"git push"`},
		`grep "git push" file`:      {`grep`, `"git push"`, `file`},
		`cat /tmp/git-push.log`:     {`cat`, `/tmp/git-push.log`},
		`git push origin main`:      {`git`, `push`, `origin`, `main`},
		`git push -f`:               {`git`, `push`, `-f`},
	}
	for input, want := range cases {
		got := tokenize(input)
		if len(got) != len(want) {
			t.Errorf("tokenize(%q): got %d tokens %v, want %d %v", input, len(got), got, len(want), want)
			continue
		}
		for i := range got {
			if got[i] != want[i] {
				t.Errorf("tokenize(%q)[%d]: got %q, want %q", input, i, got[i], want[i])
			}
		}
	}
}

// TestIsHANDSExecutePattern_BacktickSubstitution locks the invariant that
// backtick command-substitution does NOT leak inner exec patterns to the
// top level (the top-level command is still echo / printf / etc.).
// Rain msg 6418 backtick edge-case test.
func TestIsHANDSExecutePattern_BacktickSubstitution(t *testing.T) {
	cases := []string{
		"echo `git push`",
		"echo `git commit -m bad`",
		"printf '%s' `gh pr create`",
	}
	for _, cmd := range cases {
		if IsHANDSExecutePattern(cmd) {
			t.Errorf("IsHANDSExecutePattern(%q) = true, want false (backtick substitution should not leak)", cmd)
		}
	}
}

// TestIsHANDSExecutePattern_DollarParenSubstitution locks the invariant
// that $(...) command-substitution does NOT leak inner exec patterns to
// the top level. Rain msg 6418 $(...) edge-case test.
func TestIsHANDSExecutePattern_DollarParenSubstitution(t *testing.T) {
	cases := []string{
		`echo $(git push)`,
		`echo $(git commit -m bad)`,
		`echo $(gh pr create --title "X")`,
	}
	for _, cmd := range cases {
		if IsHANDSExecutePattern(cmd) {
			t.Errorf("IsHANDSExecutePattern(%q) = true, want false ($() substitution should not leak)", cmd)
		}
	}
}
