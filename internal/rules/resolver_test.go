package rules

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestDeepMerge_simpleOverride(t *testing.T) {
	base := map[string]any{"a": 1, "b": "x"}
	over := map[string]any{"b": "y", "c": 3}
	got := DeepMerge(base, over)
	want := map[string]any{"a": 1, "b": "y", "c": 3}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got %v, want %v", got, want)
	}
}

func TestDeepMerge_nestedOverride(t *testing.T) {
	base := map[string]any{
		"gates": map[string]any{
			"push": map[string]any{"requiresApproval": false},
			"forcePush": map[string]any{"blocked": false},
		},
	}
	over := map[string]any{
		"gates": map[string]any{
			"push": map[string]any{"requiresApproval": true},
		},
	}
	got := DeepMerge(base, over)
	g := got["gates"].(map[string]any)
	push := g["push"].(map[string]any)
	if push["requiresApproval"] != true {
		t.Errorf("push.requiresApproval should be true, got %v", push["requiresApproval"])
	}
	fp := g["forcePush"].(map[string]any)
	if fp["blocked"] != false {
		t.Errorf("forcePush.blocked should still be false (not lost), got %v", fp["blocked"])
	}
}

func TestDeepMerge_missingLayer(t *testing.T) {
	base := map[string]any{"x": 1}
	got := DeepMerge(base, nil)
	if got["x"] != 1 {
		t.Errorf("nil over should preserve base, got %v", got)
	}
	got = DeepMerge(nil, map[string]any{"y": 2})
	if got["y"] != 2 {
		t.Errorf("nil base should produce over, got %v", got)
	}
}

func TestDeepMerge_conflictOnLeaf(t *testing.T) {
	// Scalar replaces map (no recursion attempt).
	base := map[string]any{"k": map[string]any{"a": 1}}
	over := map[string]any{"k": "scalar"}
	got := DeepMerge(base, over)
	if got["k"] != "scalar" {
		t.Errorf("scalar should replace map, got %v", got["k"])
	}
	// Slice replaces slice (no element-wise merge).
	base = map[string]any{"list": []any{1, 2}}
	over = map[string]any{"list": []any{3}}
	got = DeepMerge(base, over)
	got_list := got["list"].([]any)
	if len(got_list) != 1 || got_list[0] != 3 {
		t.Errorf("slice should overwrite, got %v", got["list"])
	}
}

func TestResolveMaps_layers(t *testing.T) {
	general := map[string]any{
		"tone":       map[string]any{"reply": "neutral"},
		"greenlight": map[string]any{"push": "verbatim"},
	}
	project := map[string]any{
		"gates": map[string]any{"push": map[string]any{"requiresApproval": true}},
		"tone":  map[string]any{"reply": "strict"},
	}
	agent := map[string]any{
		"role": "HANDS",
	}
	merged := ResolveMaps(general, project, agent)
	tone := merged["tone"].(map[string]any)
	if tone["reply"] != "strict" {
		t.Errorf("project should override tone.reply: got %v", tone["reply"])
	}
	gl := merged["greenlight"].(map[string]any)
	if gl["push"] != "verbatim" {
		t.Errorf("general greenlight.push should survive: got %v", gl["push"])
	}
	a := merged["agent"].(map[string]any)
	if a["role"] != "HANDS" {
		t.Errorf("agent layer should land under 'agent' key: got %v", merged["agent"])
	}
}

// TestResolve_realFiles exercises Resolve() against a synthetic canonical-root
// to verify file-based loading + path fallback works.
func TestResolve_realFiles(t *testing.T) {
	root := t.TempDir()
	must := func(p, c string) {
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(c), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	must(filepath.Join(root, "rules", "general.yaml"), "tone:\n  reply: gen\n")
	must(filepath.Join(root, "projects", "p1.yaml"), "gates:\n  push:\n    requiresApproval: true\n")
	must(filepath.Join(root, "rules", "agents", "brian.yaml"), "role: HANDS\n")

	merged, err := Resolve(root, "p1", "brian")
	if err != nil {
		t.Fatal(err)
	}
	tone := merged["tone"].(map[string]any)
	if tone["reply"] != "gen" {
		t.Errorf("general tone.reply lost: %v", tone["reply"])
	}
	gates := merged["gates"].(map[string]any)
	push := gates["push"].(map[string]any)
	if push["requiresApproval"] != true {
		t.Errorf("project gates.push.requiresApproval lost: %v", push)
	}
	a := merged["agent"].(map[string]any)
	if a["role"] != "HANDS" {
		t.Errorf("agent role lost: %v", a)
	}
}

func TestResolve_missingProject_noError(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "rules"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "rules", "general.yaml"), []byte("tone:\n  reply: g\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	merged, err := Resolve(root, "nonexistent", "")
	if err != nil {
		t.Fatalf("missing project should not error: %v", err)
	}
	tone := merged["tone"].(map[string]any)
	if tone["reply"] != "g" {
		t.Errorf("general should still resolve: %v", tone)
	}
}
