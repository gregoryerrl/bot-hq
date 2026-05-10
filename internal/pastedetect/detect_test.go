// Phase T T-11 cycle-3: paste-detection tests. R39 TEST-ISOLATION via
// pure-function tests (no filesystem / DB / network).

package pastedetect

import (
	"strings"
	"testing"
)

func TestInspect_DetectsApiKeyPatterns(t *testing.T) {
	cases := []struct {
		name    string
		content string
		want    bool
	}{
		{
			name:    "deepseek_canonical_32hex",
			content: "DEEPSEEK_API_KEY=sk-deadbeefcafebabedeadbeefcafebabe",
			want:    true,
		},
		{
			name:    "anthropic_format",
			content: "ANTHROPIC_API_KEY=sk-ant-api03-aBcDeFgHiJkLmNoPqRsTuVwXyZ_abcdefg",
			want:    true,
		},
		{
			name:    "openai_format_48char",
			content: "OPENAI_API_KEY=sk-aBcDeFgHiJkLmNoPqRsTuVwXyZabcdefghijklmnopqrstuv",
			want:    true,
		},
		{
			name:    "embedded_in_paragraph",
			content: "the key is sk-1234567890abcdefghijklmnopqrstuvwxyz which I just rotated",
			want:    true,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			d := Inspect(tc.content)
			if d.Found() != tc.want {
				t.Errorf("Inspect(%q).Found() = %v, want %v", tc.content, d.Found(), tc.want)
			}
		})
	}
}

func TestInspect_DoesNotFalseFire(t *testing.T) {
	cases := []struct {
		name    string
		content string
	}{
		{
			name:    "empty",
			content: "",
		},
		{
			name:    "short_test_fixture_below_min",
			content: "DEEPSEEK_API_KEY=sk-test", // < 20 char suffix
		},
		{
			name:    "compact_pipe_peer_coord",
			content: "brian|update|standing-for-rain-BRAIN-2nd-on-T-9",
		},
		{
			name:    "msg_id_with_sk_substring_no_prefix_boundary",
			content: "task-related-skipping-this-one", // 'sk' inside word, no `sk-` prefix
		},
		{
			name:    "discord_id_numeric",
			content: "channel 1496436363761946796",
		},
		{
			name:    "git_sha_short",
			content: "HEAD=eae04a3 synced",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			d := Inspect(tc.content)
			if d.Found() {
				t.Errorf("Inspect(%q) unexpectedly fired with match=%q", tc.content, d.Match)
			}
		})
	}
}

func TestInspect_TruncatesMatchInDetection(t *testing.T) {
	full := "sk-deadbeefcafebabedeadbeefcafebabe"
	d := Inspect("DEEPSEEK_API_KEY=" + full)
	if !d.Found() {
		t.Fatal("expected detection")
	}
	if strings.Contains(d.Match, "cafebabe") || len(d.Match) > 12 {
		t.Errorf("Match = %q, expected short truncation (≤12 chars + ellipsis)", d.Match)
	}
	if !strings.HasPrefix(d.Match, "sk-") {
		t.Errorf("Match = %q, expected sk- prefix in truncation", d.Match)
	}
}

func TestFormatBlockReason_IncludesPrefixAndSuggestion(t *testing.T) {
	d := Inspect("DEEPSEEK_API_KEY=sk-1234567890abcdefghijklmnop")
	reason := FormatBlockReason(d)
	if !strings.Contains(reason, "BLOCKED") {
		t.Errorf("FormatBlockReason missing BLOCKED marker: %s", reason)
	}
	if !strings.Contains(reason, "FileVault") {
		t.Errorf("FormatBlockReason missing FileVault remediation hint: %s", reason)
	}
	if !strings.Contains(reason, "sk-") {
		t.Errorf("FormatBlockReason missing matched-prefix cite: %s", reason)
	}
}
