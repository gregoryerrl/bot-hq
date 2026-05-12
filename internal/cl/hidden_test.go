package cl

import "testing"

func TestIsHidden_TopLevelBasenamesHidden(t *testing.T) {
	cases := []string{
		"hub.db", "hub.db-shm", "hub.db-wal", "bot-hq.db",
		"webui-index.db", "live.log", "debug.log",
	}
	for _, p := range cases {
		if !IsHidden(p, false) {
			t.Errorf("expected %q to be hidden at top-level", p)
		}
	}
}

func TestIsHidden_TopLevelOnlyBasename_VoiceMirrorLog(t *testing.T) {
	if !IsHidden("voice-mirror-log.md", false) {
		t.Error("voice-mirror-log.md at top-level should be hidden")
	}
	if IsHidden("projects/bot-hq/voice-mirror-log.md", false) {
		t.Error("voice-mirror-log.md inside projects/<p>/ should NOT be hidden (top-level-only rule per plan §2.1)")
	}
	if IsHidden("projects/bcc-ad-manager/voice-mirror-log.md", false) {
		t.Error("voice-mirror-log.md inside projects/<p>/ should NOT be hidden across all projects")
	}
}

func TestIsHidden_AnyDepthBasename_LastState(t *testing.T) {
	for _, p := range []string{
		"last_state.json",
		"some/where/last_state.json",
		"projects/bot-hq/last_state.json",
	} {
		if !IsHidden(p, false) {
			t.Errorf("last_state.json at %q should be hidden at any depth", p)
		}
	}
}

func TestIsHidden_AnyDepthExtension_Database(t *testing.T) {
	for _, p := range []string{
		"foo.db",
		"projects/bot-hq/state.db",
		"x/y/z.db-wal",
	} {
		// .db-wal isn't an extension match; check basename rule via TopLevelBasenames.
		if !IsHidden(p, false) {
			t.Errorf("expected %q to be hidden by extension / basename rule", p)
		}
	}
}

func TestIsHidden_AnyDepthExtension_LogJsonl(t *testing.T) {
	for _, p := range []string{
		"some.log",
		"projects/bot-hq/audit.log",
		"diag/outbound-miss-dedup.jsonl", // diag/ also hides via TopLevelDirs
		"projects/bot-hq/findings.jsonl",
	} {
		if !IsHidden(p, false) {
			t.Errorf("expected %q to be hidden by .log/.jsonl extension", p)
		}
	}
}

func TestIsHidden_TopLevelDirs_BlockSubtree(t *testing.T) {
	cases := []struct {
		path  string
		isDir bool
	}{
		{"diag", true},
		{"sentinels", true},
		{"bridge", true},
		{"plugins", true},
		{"gates", true},
		{"sessions", true},
		{"diag/outbound-miss.jsonl", false},
		{"sentinels/docdrift.log", false},
		{"plugins/github/README.md", false},
		{"gates/pre-commit-checklist.md", false},
		{"sessions/2026-05-12-cl-uniformity/manifest.md", false},
	}
	for _, c := range cases {
		if !IsHidden(c.path, c.isDir) {
			t.Errorf("expected %q (isDir=%v) to be hidden via TopLevelDirs rule", c.path, c.isDir)
		}
	}
}

func TestIsHidden_AgentDirs_TopLevelEntryAllowed(t *testing.T) {
	for _, agent := range []string{"brian", "rain", "emma", "clive"} {
		if IsHidden(agent, true) {
			t.Errorf("agent dir %q at top-level should NOT be hidden (tree-walker enters for allowlisted subpaths)", agent)
		}
	}
}

func TestIsHidden_AgentDirs_ContentsDenyByDefault(t *testing.T) {
	for _, p := range []string{
		"brian/last_state.json",
		"rain/scratch.md",
		"emma/notes.txt",
		"clive/secret.yaml",
	} {
		if !IsHidden(p, false) {
			t.Errorf("expected %q to be hidden by agent-dir deny-by-default", p)
		}
	}
}

func TestIsHidden_AgentSubpathAllowlist_DisciplineAnchors(t *testing.T) {
	for _, p := range []string{
		"brian/discipline-anchors.md",
		"rain/discipline-anchors.md",
		"emma/discipline-anchors.md",
	} {
		if IsHidden(p, false) {
			t.Errorf("expected %q to NOT be hidden (allowlisted under agent dir)", p)
		}
	}
}

func TestIsHidden_Dotfiles(t *testing.T) {
	for _, p := range []string{
		".git",
		".DS_Store",
		"projects/bot-hq/.cache",
		"some/.hidden/foo.md",
	} {
		if !IsHidden(p, false) {
			t.Errorf("expected dotfile %q to be hidden", p)
		}
	}
}

func TestIsHidden_AllowedPaths(t *testing.T) {
	for _, p := range []string{
		"README.md",
		"glossary.md",
		"projects/bot-hq/README.md",
		"projects/bot-hq/INDEX.md",
		"projects/bot-hq/architecture/sessions-as-containers.md",
		"projects/bot-hq/phase/phase-z.md",
		"projects/bcc-ad-manager/clips/2026-05-06-task-reply-draft.md",
		"discipline-log.md",
		"tasks.md",
		"agent-onboarding.md",
		"rulebook.md",
	} {
		if IsHidden(p, false) {
			t.Errorf("expected %q to NOT be hidden", p)
		}
	}
}

func TestIsHidden_EmptyPath(t *testing.T) {
	if IsHidden("", false) {
		t.Error("empty path should NOT be hidden")
	}
	if IsHidden(".", true) {
		t.Error("root path '.' should NOT be hidden")
	}
}

