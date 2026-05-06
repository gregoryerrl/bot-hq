package tasks

import (
	"testing"
)

func TestDecode_basicTasks(t *testing.T) {
	raw := `---
tasks:
  - id: t1
    title: First task
    status: pending
    owner: brian
  - id: t2
    title: Second
    status: done
---

# Body description

Some context here.
`
	f, err := Decode(raw)
	if err != nil {
		t.Fatal(err)
	}
	if len(f.Frontmatter.Tasks) != 2 {
		t.Fatalf("expected 2 tasks, got %d", len(f.Frontmatter.Tasks))
	}
	if f.Frontmatter.Tasks[0].ID != "t1" {
		t.Errorf("task[0].id = %q", f.Frontmatter.Tasks[0].ID)
	}
	if f.Frontmatter.Tasks[1].Status != "done" {
		t.Errorf("task[1].status = %q", f.Frontmatter.Tasks[1].Status)
	}
}

func TestValidate_warnsBadStatus(t *testing.T) {
	f := &File{Frontmatter: Frontmatter{Tasks: []Task{
		{ID: "t1", Title: "ok", Status: "pending"},
		{ID: "t2", Title: "bad", Status: "wibble"},
		{Title: "no id", Status: "done"},
	}}}
	warns := f.Validate()
	if len(warns) < 2 {
		t.Errorf("expected >=2 warnings (bad status + missing id), got %v", warns)
	}
}

func TestWriteRead_roundTrip(t *testing.T) {
	root := t.TempDir()
	in := File{
		Frontmatter: Frontmatter{Tasks: []Task{{ID: "t1", Title: "x", Status: "pending"}}},
		Body:        "# Notes\n",
	}
	if err := Write(root, "p1", in); err != nil {
		t.Fatal(err)
	}
	got, err := Read(root, "p1")
	if err != nil {
		t.Fatal(err)
	}
	if got == nil || len(got.Frontmatter.Tasks) != 1 {
		t.Fatalf("read failed: %+v", got)
	}
	if got.Frontmatter.Tasks[0].ID != "t1" {
		t.Errorf("task id lost: %q", got.Frontmatter.Tasks[0].ID)
	}
}

func TestRead_missing_returnsNilNil(t *testing.T) {
	root := t.TempDir()
	got, err := Read(root, "absent")
	if err != nil {
		t.Fatalf("missing should not error: %v", err)
	}
	if got != nil {
		t.Errorf("missing should return nil: %+v", got)
	}
}
