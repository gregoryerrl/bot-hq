package snap

import (
	"strings"
	"testing"
)

func TestFormatCanonical(t *testing.T) {
	s := SNAP{
		Branches: []string{"bot-hq:main@abc1234"},
		Agents:   []string{"brian(idle)"},
		Pending:  "user input",
		Next:     "ship it",
	}
	got := s.Format()
	want := "SNAP:\n" +
		"Branches: bot-hq:main@abc1234\n" +
		"Agents:   brian(idle)\n" +
		"Pending:  user input\n" +
		"Next:     ship it"
	if got != want {
		t.Errorf("Format()\n got:\n%q\nwant:\n%q", got, want)
	}
}

func TestRoundTripCanonical(t *testing.T) {
	cases := []struct {
		name string
		in   SNAP
	}{
		{
			name: "single items",
			in: SNAP{
				Branches: []string{"bot-hq:main@abc1234"},
				Agents:   []string{"brian(idle)"},
				Pending:  "user input",
				Next:     "ship it",
			},
		},
		{
			name: "multi-item lists with parens",
			in: SNAP{
				Branches: []string{"bot-hq:main@9b17042", "brian/phase-g-v1-slice-2@9b17042 (C1 in flight)"},
				Agents:   []string{"brian(C1 in flight)", "rain(diff-gate armed)", "emma(nominal)"},
				Pending:  "C1 implementation",
				Next:     "C1 land → Rain diff-gate → C2",
			},
		},
		{
			name: "all-empty",
			in: SNAP{
				Branches: nil,
				Agents:   nil,
				Pending:  "",
				Next:     "",
			},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			wire := tc.in.Format()
			got, err := Parse(wire)
			if err != nil {
				t.Fatalf("Parse(Format(s)) err: %v", err)
			}
			if !snapEqual(got, tc.in) {
				t.Errorf("round-trip mismatch\n got: %#v\nwant: %#v", got, tc.in)
			}
		})
	}
}

// TestRegressionMsg3122 pins the comma-in-parens hazard surfaced by Brian's
// own SNAP at hub message 3122 (slice 2 design discussion). Naive comma-split
// would fragment the branch list on the comma inside
// "(slice 1.5 + followup live, awaiting rebuild to render border)". Failing
// this test = parser regression on the v1 escape mechanism (paren-depth).
func TestRegressionMsg3122(t *testing.T) {
	body := `preamble text that should be ignored

SNAP:
Branches: bot-hq:main@9b17042 (slice 1.5 + followup live, awaiting rebuild to render border)
Agents:   brian(holding for all-clear), rain(SQL belt in flight, ETA-pinged), emma(nominal)
Pending:  Rain SQL belt result → my all-clear → user fires rebuild #11
Next:     SQL belt PASS → all-clear broadcast → rebuild #11`

	got, err := Parse(body)
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	if len(got.Branches) != 1 {
		t.Errorf("Branches: got %d items, want 1 (paren-aware split must not fragment)", len(got.Branches))
	}
	if !strings.Contains(got.Branches[0], "awaiting rebuild") {
		t.Errorf("Branches[0] truncated: %q", got.Branches[0])
	}
	if len(got.Agents) != 3 {
		t.Errorf("Agents: got %d items, want 3", len(got.Agents))
	}
}

func TestParseErrors(t *testing.T) {
	cases := []struct {
		name string
		body string
		want error
	}{
		{
			name: "no SNAP marker",
			body: "just some text without the marker",
			want: ErrNoSNAPBlock,
		},
		{
			name: "truncated block",
			body: "SNAP:\nBranches: x\nAgents:   y",
			want: ErrMalformedFields,
		},
		{
			name: "fields out of order",
			body: "SNAP:\nAgents:   a\nBranches: b\nPending:  p\nNext:     n",
			want: ErrMalformedFields,
		},
		{
			name: "wrong label",
			body: "SNAP:\nBranches: b\nAgents:   a\nBlocked:  p\nNext:     n",
			want: ErrMalformedFields,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := Parse(tc.body)
			if err != tc.want {
				t.Errorf("err = %v, want %v", err, tc.want)
			}
		})
	}
}

// TestParseCRLFTolerant locks that CRLF line endings parse equivalently to
// LF. Hub messages are LF-only on the wire, but defensive: if a SNAP arrives
// with CRLF (e.g. pasted from Windows / mid-network conversion), TrimSpace
// in Parse strips \r and the block still resolves cleanly. Per Rain C1
// diff-gate observation #3.
func TestParseCRLFTolerant(t *testing.T) {
	body := "SNAP:\r\n" +
		"Branches: bot-hq:main@abc1234\r\n" +
		"Agents:   brian(idle)\r\n" +
		"Pending:  none\r\n" +
		"Next:     ship"
	got, err := Parse(body)
	if err != nil {
		t.Fatalf("Parse(CRLF body): %v", err)
	}
	if got.Pending != "none" || got.Next != "ship" {
		t.Errorf("CRLF \\r leaked into trimmed values: pending=%q next=%q", got.Pending, got.Next)
	}
	if len(got.Branches) != 1 || got.Branches[0] != "bot-hq:main@abc1234" {
		t.Errorf("CRLF mangled Branches: %#v", got.Branches)
	}
}

// TestSplitDepth0Malformed documents the contract for malformed paren input.
// Unclosed `(` keeps depth>0 forever → the entire string returns as one item.
// Unmatched `)` is guarded by `if depth > 0` → no underflow, splits at the
// outer `,` as if the rogue `)` were literal. Both cases produce no panic;
// the parser is permissive on malformed input. Per Rain C1 obs #4.
func TestSplitDepth0Malformed(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want []string
	}{
		{"unclosed paren swallows everything", "a(x, b", []string{"a(x, b"}},
		{"unmatched close paren splits normally", "a), b", []string{"a)", "b"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := splitDepth0(tc.in)
			if !stringSliceEqual(got, tc.want) {
				t.Errorf("splitDepth0(%q) = %#v, want %#v", tc.in, got, tc.want)
			}
		})
	}
}

func TestSplitDepth0(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want []string
	}{
		{"empty", "", nil},
		{"single", "a", []string{"a"}},
		{"two", "a, b", []string{"a", "b"}},
		{"comma in parens", "a(x, y), b", []string{"a(x, y)", "b"}},
		{"nested parens with comma", "a((nested, comma)), b", []string{"a((nested, comma))", "b"}},
		{"single nested", "agent(holding for shape-call (msg 3130))", []string{"agent(holding for shape-call (msg 3130))"}},
		{"three with embedded commas", "x(a, b), y(c, d, e), z", []string{"x(a, b)", "y(c, d, e)", "z"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := splitDepth0(tc.in)
			if !stringSliceEqual(got, tc.want) {
				t.Errorf("splitDepth0(%q) = %#v, want %#v", tc.in, got, tc.want)
			}
		})
	}
}

func snapEqual(a, b SNAP) bool {
	return stringSliceEqual(a.Branches, b.Branches) &&
		stringSliceEqual(a.Agents, b.Agents) &&
		a.Pending == b.Pending &&
		a.Next == b.Next
}

func stringSliceEqual(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}
