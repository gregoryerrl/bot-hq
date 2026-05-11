package projects

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"gopkg.in/yaml.v3"
)

// Phase Z CL-uniformity S1: tests for Rules.Extensions + ValidateExtensions.

func TestExtensions_Unmarshal_FullBlock(t *testing.T) {
	src := `project_name: "bot-hq"
remote_url: "https://github.com/gregoryerrl/bot-hq.git"
extensions:
  universal_opt_in:
    - env
  external_docs_pointer:
    - arcs-index.md
    - conventions-index.md
  brain_duo_operational:
    - phase
    - ratchets
    - discipline-log.md
    - voice-mirror-log.md
  foundational_anchors:
    - vision.md
    - bootstrap.md
`
	var r Rules
	if err := yaml.Unmarshal([]byte(src), &r); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got, want := r.ProjectName, "bot-hq"; got != want {
		t.Errorf("ProjectName: got %q, want %q", got, want)
	}
	if got, want := len(r.Extensions.UniversalOptIn), 1; got != want {
		t.Errorf("UniversalOptIn len: got %d, want %d", got, want)
	}
	if got, want := r.Extensions.ExternalDocsPointer, []string{"arcs-index.md", "conventions-index.md"}; !equalSlices(got, want) {
		t.Errorf("ExternalDocsPointer: got %v, want %v", got, want)
	}
	if got, want := r.Extensions.BrainDuoOperational, []string{"phase", "ratchets", "discipline-log.md", "voice-mirror-log.md"}; !equalSlices(got, want) {
		t.Errorf("BrainDuoOperational: got %v, want %v", got, want)
	}
	if got, want := r.Extensions.FoundationalAnchors, []string{"vision.md", "bootstrap.md"}; !equalSlices(got, want) {
		t.Errorf("FoundationalAnchors: got %v, want %v", got, want)
	}
}

func TestExtensions_Unmarshal_AbsentBlock(t *testing.T) {
	src := `project_name: "988"
remote_url: ""
`
	var r Rules
	if err := yaml.Unmarshal([]byte(src), &r); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	// Absent extensions = zero-value (nil slices), no error.
	if r.Extensions.UniversalOptIn != nil {
		t.Errorf("UniversalOptIn: got %v, want nil", r.Extensions.UniversalOptIn)
	}
	if r.Extensions.BrainDuoOperational != nil {
		t.Errorf("BrainDuoOperational: got %v, want nil", r.Extensions.BrainDuoOperational)
	}
}

func TestExtensions_Unmarshal_PartialBlock(t *testing.T) {
	src := `project_name: "bcc-ad-manager"
extensions:
  universal_opt_in:
    - env
`
	var r Rules
	if err := yaml.Unmarshal([]byte(src), &r); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got, want := r.Extensions.UniversalOptIn, []string{"env"}; !equalSlices(got, want) {
		t.Errorf("UniversalOptIn: got %v, want %v", got, want)
	}
	if r.Extensions.BrainDuoOperational != nil {
		t.Errorf("BrainDuoOperational should be nil for non-bot-hq with no declaration")
	}
}

func TestExtensions_AllNames(t *testing.T) {
	e := ExtensionsBlock{
		UniversalOptIn:      []string{"env"},
		ExternalDocsPointer: []string{"arcs-index.md"},
		BrainDuoOperational: []string{"phase", "ratchets"},
		FoundationalAnchors: []string{"vision.md"},
	}
	got := e.AllNames()
	want := []string{"env", "arcs-index.md", "phase", "ratchets", "vision.md"}
	if !equalSlices(got, want) {
		t.Errorf("AllNames: got %v, want %v", got, want)
	}
}

func TestValidateExtensions_BrainDuoOperational_NonBotHQ_Error(t *testing.T) {
	r := Rules{
		ProjectName: "bcc-ad-manager",
		Extensions: ExtensionsBlock{
			BrainDuoOperational: []string{"phase", "ratchets"},
		},
	}
	errs := r.ValidateExtensions("")
	if len(errs) != 2 {
		t.Fatalf("want 2 errors (one per declared name), got %d: %v", len(errs), errs)
	}
	for _, e := range errs {
		if e.Severity != "error" {
			t.Errorf("severity: got %q, want error", e.Severity)
		}
		if e.Class != "brain_duo_operational" {
			t.Errorf("class: got %q, want brain_duo_operational", e.Class)
		}
	}
}

func TestValidateExtensions_BrainDuoOperational_BotHQ_OK(t *testing.T) {
	r := Rules{
		ProjectName: "bot-hq",
		Extensions: ExtensionsBlock{
			BrainDuoOperational: []string{"phase", "ratchets", "discipline-log.md"},
		},
	}
	errs := r.ValidateExtensions("")
	// No rule-1 errors; basenames all valid; no duplicates.
	for _, e := range errs {
		if e.Severity == "error" {
			t.Errorf("unexpected error: %v", e)
		}
	}
}

func TestValidateExtensions_BadBasename_Error(t *testing.T) {
	cases := map[string]string{
		"path-traversal":     "../etc/passwd",
		"leading-dot":        ".hidden",
		"leading-slash":      "/abs/path",
		"trailing-dot":       "foo.",
		"empty":              "",
		"uppercase":          "Foo.md",
		"consecutive-dots":   "foo..md",
		"single-letter-only": "a",
	}
	for label, bad := range cases {
		t.Run(label, func(t *testing.T) {
			r := Rules{
				ProjectName: "988",
				Extensions: ExtensionsBlock{
					UniversalOptIn: []string{bad},
				},
			}
			errs := r.ValidateExtensions("")
			if label == "single-letter-only" {
				// "a" actually matches the regex (^[a-z][a-z0-9-]*$ with empty
				// optional extension); skip — this isn't an invalid case.
				return
			}
			found := false
			for _, e := range errs {
				if e.Severity == "error" && e.Name == bad && e.Class == "universal_opt_in" {
					found = true
				}
			}
			if !found {
				t.Errorf("want error for bad basename %q (%s); got errs=%v", bad, label, errs)
			}
		})
	}
}

func TestValidateExtensions_ValidBasenames_NoError(t *testing.T) {
	r := Rules{
		ProjectName: "bot-hq",
		Extensions: ExtensionsBlock{
			UniversalOptIn:      []string{"env"},
			ExternalDocsPointer: []string{"arcs-index.md", "conventions-index.md"},
			BrainDuoOperational: []string{"phase", "ratchets", "discipline-log.md", "voice-mirror-log.md"},
			FoundationalAnchors: []string{"vision.md", "bootstrap.md"},
		},
	}
	errs := r.ValidateExtensions("")
	for _, e := range errs {
		if e.Severity == "error" {
			t.Errorf("unexpected error for valid basenames: %v", e)
		}
	}
}

func TestValidateExtensions_DuplicateBasename_Error(t *testing.T) {
	r := Rules{
		ProjectName: "bot-hq",
		Extensions: ExtensionsBlock{
			UniversalOptIn:      []string{"env"},
			ExternalDocsPointer: []string{"env"}, // duplicate
		},
	}
	errs := r.ValidateExtensions("")
	found := false
	for _, e := range errs {
		if e.Severity == "error" && e.Name == "env" {
			found = true
		}
	}
	if !found {
		t.Errorf("want duplicate-basename error for %q in two classes; got errs=%v", "env", errs)
	}
}

func TestValidateExtensions_DeclaredNotOnDisk_Warning(t *testing.T) {
	// R39 TEST-ISOLATION: use t.TempDir() — never touch real CL state.
	tmp := t.TempDir()
	// Layout: <tmp>/projects/foo/ with only vision.md on disk (phase declared but absent)
	projDir := filepath.Join(tmp, "projects", "foo")
	if err := os.MkdirAll(projDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "vision.md"), []byte("x"), 0o644); err != nil {
		t.Fatalf("write vision: %v", err)
	}

	r := Rules{
		ProjectName: "foo",
		Extensions: ExtensionsBlock{
			FoundationalAnchors: []string{"vision.md", "bootstrap.md"}, // bootstrap absent
		},
	}
	errs := r.ValidateExtensions(tmp)

	var warnings []ExtensionsValidationError
	for _, e := range errs {
		if e.Severity == "warning" {
			warnings = append(warnings, e)
		}
	}
	if len(warnings) != 1 {
		t.Fatalf("want 1 warning (bootstrap.md absent), got %d: %v", len(warnings), warnings)
	}
	if warnings[0].Name != "bootstrap.md" {
		t.Errorf("warning name: got %q, want bootstrap.md", warnings[0].Name)
	}
}

func TestValidateExtensions_EmptyCanonRoot_NoOnDiskCheck(t *testing.T) {
	r := Rules{
		ProjectName: "foo",
		Extensions: ExtensionsBlock{
			FoundationalAnchors: []string{"never-exists.md"},
		},
	}
	errs := r.ValidateExtensions("") // empty canonRoot skips Rule 4
	for _, e := range errs {
		if e.Severity == "warning" {
			t.Errorf("unexpected warning when canonRoot empty: %v", e)
		}
	}
}

func TestExtensions_Roundtrip_4Projects(t *testing.T) {
	cases := map[string]Rules{
		"988": {
			ProjectName: "988",
			RemoteURL:   "",
			Extensions:  ExtensionsBlock{}, // canonical-only
		},
		"bcc-ad-manager": {
			ProjectName: "bcc-ad-manager",
			RemoteURL:   "https://github.com/example/bcc-ad-manager.git",
			Extensions: ExtensionsBlock{
				UniversalOptIn: []string{"env"},
			},
		},
		"boncom-labs-live-on-playbooks": {
			ProjectName: "boncom-labs-live-on-playbooks",
			RemoteURL:   "",
			Extensions:  ExtensionsBlock{}, // canonical-only
		},
		"bot-hq": {
			ProjectName: "bot-hq",
			RemoteURL:   "https://github.com/gregoryerrl/bot-hq.git",
			Extensions: ExtensionsBlock{
				UniversalOptIn:      []string{"env"},
				ExternalDocsPointer: []string{"arcs-index.md", "conventions-index.md"},
				BrainDuoOperational: []string{"phase", "ratchets", "discipline-log.md", "voice-mirror-log.md"},
				FoundationalAnchors: []string{"vision.md", "bootstrap.md"},
			},
		},
	}
	for name, in := range cases {
		t.Run(name, func(t *testing.T) {
			out, err := yaml.Marshal(&in)
			if err != nil {
				t.Fatalf("marshal: %v", err)
			}
			var rt Rules
			if err := yaml.Unmarshal(out, &rt); err != nil {
				t.Fatalf("unmarshal: %v", err)
			}
			if rt.ProjectName != in.ProjectName {
				t.Errorf("ProjectName drift: got %q, want %q", rt.ProjectName, in.ProjectName)
			}
			if !equalSlices(rt.Extensions.UniversalOptIn, in.Extensions.UniversalOptIn) {
				t.Errorf("UniversalOptIn drift: got %v, want %v", rt.Extensions.UniversalOptIn, in.Extensions.UniversalOptIn)
			}
			if !equalSlices(rt.Extensions.BrainDuoOperational, in.Extensions.BrainDuoOperational) {
				t.Errorf("BrainDuoOperational drift: got %v, want %v", rt.Extensions.BrainDuoOperational, in.Extensions.BrainDuoOperational)
			}
		})
	}
}

func equalSlices(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	aa := append([]string(nil), a...)
	bb := append([]string(nil), b...)
	sort.Strings(aa)
	sort.Strings(bb)
	for i := range aa {
		if aa[i] != bb[i] {
			return false
		}
	}
	return true
}
