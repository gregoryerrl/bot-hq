// Package cl — index.go: Phase Y-1 project INDEX.md generator.
//
// Walks a project's CL substrate + emits a discoverability map. Format:
// YAML frontmatter (machine-friendly) + Markdown body (human-friendly).
//
// Substrate convention (declared in projects/<p>.yaml library.schema):
//
//   <project>/
//     README.md          # user-voice project overview
//     INDEX.md           # this file (auto-generated)
//     architecture/      # durable system docs
//     decisions/         # ADRs (date-prefixed)
//     conventions/       # coding/git conventions
//     glossary/          # domain terms
//     audit-notes/       # past audit findings + open known issues
//     plans/             # ad-hoc plan docs (non-IPAV-task scoped)
//     tasks/             # IPAV per-task subtree (managed by hub_ipav_*)
//     eod/               # user-facing EOD clips
//     clips/             # user-message clips
//
// Bot-hq self-substrate omits plans/eod/clips by design (those fold to
// canonical-store top-level: phase/, per-session, n/a).

package cl

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

// IndexFrontmatter is the YAML head-block of a project INDEX.md.
// TotalArtifacts is project-level only; the cross-project top-level INDEX
// omits it via omitempty since per-project granularity is the right
// scope for that count.
type IndexFrontmatter struct {
	GeneratedAt        time.Time `yaml:"generated_at"`
	Project            string    `yaml:"project"`
	Generator          string    `yaml:"generator"`
	SchemaVersion      int       `yaml:"schema_version"`
	TotalArtifacts     int       `yaml:"total_artifacts,omitempty"`
	IPAVTasksOpen      int       `yaml:"ipav_tasks_open"`
	IPAVTasksClosed30d int       `yaml:"ipav_tasks_closed_30d"`
}

// IndexSchemaVersion bumps when frontmatter shape changes incompatibly.
const IndexSchemaVersion = 1

// Subdir classes recognized by the indexer. Order is rendering order.
var indexSubdirs = []string{
	"architecture",
	"decisions",
	"conventions",
	"glossary",
	"audit-notes",
	"plans",
	"tasks",
	"eod",
	"clips",
}

// subdirDescription is the one-liner used in the summary table for each
// known subdir class. Keeps the rendering self-documenting even when the
// per-subdir README isn't authored yet.
var subdirDescription = map[string]string{
	"architecture": "Bot-hq's mental model of the system.",
	"decisions":    "Architecture Decision Records (ADRs).",
	"conventions":  "Project-specific coding/git conventions.",
	"glossary":     "Domain terms + acronyms.",
	"audit-notes":  "Past audit findings + open known issues.",
	"plans":        "In-flight + completed planning docs.",
	"tasks":        "IPAV per-task subtree (managed by hub_ipav_*).",
	"eod":          "End-of-day clips (user-facing).",
	"clips":        "User-message + paste-back clips.",
}

// IndexProject regenerates ~/.bot-hq/projects/<project>/INDEX.md from
// disk state. Returns the rendered markdown + a bool indicating whether
// the on-disk INDEX.md actually changed (false → no-op write avoided).
func (c *CL) IndexProject(project string) (string, bool, error) {
	if project == "" {
		return "", false, fmt.Errorf("project required")
	}
	projDir := filepath.Join(c.root, "projects", project)
	if _, err := os.Stat(projDir); err != nil {
		if os.IsNotExist(err) {
			return "", false, fmt.Errorf("%w: project dir %s", ErrNotFound, projDir)
		}
		return "", false, err
	}

	rendered, err := c.renderProjectIndex(project, projDir)
	if err != nil {
		return "", false, err
	}

	indexPath := filepath.Join(projDir, "INDEX.md")
	prev, err := os.ReadFile(indexPath)
	if err != nil && !os.IsNotExist(err) {
		return "", false, err
	}
	// Idempotency: ignore the generated_at line when comparing — it always
	// changes. Compare the rest.
	if bytesEqualIgnoringFrontmatterTimestamp(prev, []byte(rendered)) {
		return rendered, false, nil
	}

	tmp := indexPath + ".tmp"
	if err := os.WriteFile(tmp, []byte(rendered), 0o644); err != nil {
		return "", false, err
	}
	if err := os.Rename(tmp, indexPath); err != nil {
		_ = os.Remove(tmp)
		return "", false, err
	}
	return rendered, true, nil
}

// renderProjectIndex builds the INDEX.md content. Pure function over disk
// state — no writes. Caller (IndexProject) handles atomic write.
func (c *CL) renderProjectIndex(project, projDir string) (string, error) {
	type subdirSummary struct {
		Name         string
		Files        []os.DirEntry
		LastModified time.Time
	}

	summaries := make([]subdirSummary, 0, len(indexSubdirs))
	totalArtifacts := 0
	for _, sub := range indexSubdirs {
		entries, last, err := readSubdir(filepath.Join(projDir, sub))
		if err != nil {
			return "", fmt.Errorf("scan %s: %w", sub, err)
		}
		// Filter to .md/.sql/.json/.yaml + skip README.md (substrate doc, not content)
		filtered := make([]os.DirEntry, 0, len(entries))
		for _, e := range entries {
			n := e.Name()
			if n == "README.md" || strings.HasPrefix(n, ".") {
				continue
			}
			filtered = append(filtered, e)
		}
		summaries = append(summaries, subdirSummary{
			Name:         sub,
			Files:        filtered,
			LastModified: last,
		})
		totalArtifacts += len(filtered)
	}

	openTasks, closed30dTasks, err := c.scanIPAVTasks(project, projDir)
	if err != nil {
		return "", fmt.Errorf("scan IPAV tasks: %w", err)
	}

	fm := IndexFrontmatter{
		GeneratedAt:        time.Now().UTC(),
		Project:            project,
		Generator:          "bot-hq cl-index",
		SchemaVersion:      IndexSchemaVersion,
		TotalArtifacts:     totalArtifacts,
		IPAVTasksOpen:      len(openTasks),
		IPAVTasksClosed30d: len(closed30dTasks),
	}

	var b strings.Builder
	if err := writeFrontmatter(&b, fm); err != nil {
		return "", err
	}

	fmt.Fprintf(&b, "# %s — library index\n\n", project)
	b.WriteString("Auto-generated discoverability map. Regenerate via `bot-hq cl-index ")
	b.WriteString(project)
	b.WriteString("`.\n\n")

	// Project rules — load-bearing pointer for the BRAIN-duo. Read BEFORE any
	// HANDS-class action; gates encoded here are how per-project rule-strictness
	// gets resolved (e.g., bot-hq lenient vs bcc-ad-manager strict push gate).
	fmt.Fprintf(&b, "## Project rules\n\n")
	fmt.Fprintf(&b, "Canonical rules file: [`projects/%s.yaml`](../%s.yaml)\n\n", project, project)
	b.WriteString("Read this BEFORE any HANDS-class action (commit / push / merge). Fields you need:\n\n")
	b.WriteString("- `branch.{pattern, examples, patternHelp}` — branch naming convention\n")
	b.WriteString("- `gates.push.requiresApproval` — when `false`, Rain BRAIN-2nd alone is sufficient for commit + push within session scope (e.g. bot-hq). When `true`, per-instance user verbatim still required (e.g. bcc-ad-manager).\n")
	b.WriteString("- `gates.forcePush.{blocked, tokenFormat}` — force-push lock + user-token format\n")
	b.WriteString("- `gates.coder.{toolsBlocked, perActionApproval}` — coder subagent constraints\n")
	b.WriteString("- `commit.{style, requireIssueLink}` — commit message shape\n")
	b.WriteString("- `project_feedback.*` — project-specific behavioral rules (load-bearing per-feedback cite-anchor)\n\n")

	// Summary table
	b.WriteString("## Summary\n\n")
	b.WriteString("| Class | Count | Last activity |\n")
	b.WriteString("|-------|------:|---------------|\n")
	for _, s := range summaries {
		last := "(empty)"
		if !s.LastModified.IsZero() {
			last = s.LastModified.UTC().Format("2006-01-02")
		}
		fmt.Fprintf(&b, "| %s | %d | %s |\n", s.Name, len(s.Files), last)
	}
	b.WriteString("\n")

	// IPAV tasks
	b.WriteString("## Active IPAV tasks\n\n")
	if len(openTasks) == 0 {
		b.WriteString("(none)\n\n")
	} else {
		b.WriteString("| Task ID | Phase | Decision | Opened |\n")
		b.WriteString("|---------|-------|----------|--------|\n")
		for _, t := range openTasks {
			fmt.Fprintf(&b, "| %s | %s | %s | %s |\n", t.TaskID, t.CurrentPhase, t.DecisionClass, t.OpenedAt.UTC().Format("2006-01-02"))
		}
		b.WriteString("\n")
	}

	b.WriteString("## Recently closed IPAV tasks (last 30 days)\n\n")
	if len(closed30dTasks) == 0 {
		b.WriteString("(none)\n\n")
	} else {
		b.WriteString("| Task ID | Result | Closed |\n")
		b.WriteString("|---------|--------|--------|\n")
		for _, t := range closed30dTasks {
			closedAt := t.ClosedAt.UTC().Format("2006-01-02")
			fmt.Fprintf(&b, "| %s | %s | %s |\n", t.TaskID, t.Result, closedAt)
		}
		b.WriteString("\n")
	}

	// Substrate by class
	b.WriteString("## Substrate by class\n\n")
	for _, s := range summaries {
		desc := subdirDescription[s.Name]
		fmt.Fprintf(&b, "### %s/\n", s.Name)
		if desc != "" {
			fmt.Fprintf(&b, "*%s*", desc)
			readmePath := filepath.Join(projDir, s.Name, "README.md")
			if _, err := os.Stat(readmePath); err == nil {
				fmt.Fprintf(&b, " See `%s/README.md` for filename convention.", s.Name)
			}
			b.WriteString("\n\n")
		}
		if s.Name == "tasks" {
			// IPAV tasks rendered above; subdir listing here would duplicate.
			b.WriteString("(see Active IPAV tasks + Recently closed sections above)\n\n")
			continue
		}
		if len(s.Files) == 0 {
			b.WriteString("(empty)\n\n")
			continue
		}
		b.WriteString("| File | Size | Modified |\n")
		b.WriteString("|------|-----:|----------|\n")
		// Sort by modified-time desc so freshest is at the top
		entriesSorted := make([]os.DirEntry, len(s.Files))
		copy(entriesSorted, s.Files)
		sort.Slice(entriesSorted, func(i, j int) bool {
			ai, _ := entriesSorted[i].Info()
			aj, _ := entriesSorted[j].Info()
			return ai.ModTime().After(aj.ModTime())
		})
		for _, e := range entriesSorted {
			info, err := e.Info()
			if err != nil {
				continue
			}
			fmt.Fprintf(&b, "| %s | %s | %s |\n", e.Name(), formatSize(info.Size()), info.ModTime().UTC().Format("2006-01-02"))
		}
		b.WriteString("\n")
	}

	// External docs cross-link
	if extRoot := externalDocsRoot(c.root, project); extRoot != "" {
		b.WriteString("## Related external docs\n\n")
		fmt.Fprintf(&b, "External docs root: `%s` (read-only mirror in webui \"Project docs\" destination)\n\n", extRoot)
		expanded := expandHome(extRoot)
		entries, err := os.ReadDir(expanded)
		if err == nil && len(entries) > 0 {
			b.WriteString("| File |\n|------|\n")
			for _, e := range entries {
				if strings.HasPrefix(e.Name(), ".") {
					continue
				}
				suffix := ""
				if e.IsDir() {
					suffix = "/"
				}
				fmt.Fprintf(&b, "| %s%s |\n", e.Name(), suffix)
			}
			b.WriteString("\n")
		}
	}

	b.WriteString("## Cross-project IPAV index\n\n")
	b.WriteString("See `~/.bot-hq/INDEX.md` for active + recently-closed IPAV tasks across all projects.\n")

	return b.String(), nil
}

// IPAVTaskRow is a row in the cross-project IPAV index. Fields kept
// minimal — the per-task subtree carries the full record.
type IPAVTaskRow struct {
	Project       string
	TaskID        string
	CurrentPhase  string
	DecisionClass string
	OpenedAt      time.Time
	ClosedAt      time.Time
	Result        string
}

// scanIPAVTasks walks projects/<project>/tasks/<task-id>/ipav-state.yaml
// and partitions into (open, closed-within-30-days).
func (c *CL) scanIPAVTasks(project, projDir string) (open, recent []IPAVTaskRow, err error) {
	tasksDir := filepath.Join(projDir, "tasks")
	entries, err := os.ReadDir(tasksDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil, nil
		}
		return nil, nil, err
	}
	cutoff := time.Now().Add(-30 * 24 * time.Hour)
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		taskID := e.Name()
		row, ok, err := loadIPAVTaskRow(project, filepath.Join(tasksDir, taskID))
		if err != nil {
			return nil, nil, err
		}
		if !ok {
			continue
		}
		row.TaskID = taskID
		if row.ClosedAt.IsZero() {
			open = append(open, row)
		} else if row.ClosedAt.After(cutoff) {
			recent = append(recent, row)
		}
	}
	sort.Slice(open, func(i, j int) bool { return open[i].OpenedAt.After(open[j].OpenedAt) })
	sort.Slice(recent, func(i, j int) bool { return recent[i].ClosedAt.After(recent[j].ClosedAt) })
	return open, recent, nil
}

// loadIPAVTaskRow reads ipav-state.yaml and projects to a flat row.
// Returns ok=false if the state file is missing (orphaned task dir).
func loadIPAVTaskRow(project, taskDir string) (IPAVTaskRow, bool, error) {
	statePath := filepath.Join(taskDir, "ipav-state.yaml")
	data, err := os.ReadFile(statePath)
	if err != nil {
		if os.IsNotExist(err) {
			return IPAVTaskRow{}, false, nil
		}
		return IPAVTaskRow{}, false, err
	}
	var raw struct {
		TaskID         string    `yaml:"task_id"`
		CurrentPhase   string    `yaml:"current_phase"`
		DecisionClass  string    `yaml:"decision_class"`
		OpenedAt       time.Time `yaml:"opened_at"`
		ClosedAt       time.Time `yaml:"closed_at"`
		VerifyResult   string    `yaml:"verify_result"`
	}
	if err := yaml.Unmarshal(data, &raw); err != nil {
		return IPAVTaskRow{}, false, fmt.Errorf("parse %s: %w", statePath, err)
	}
	return IPAVTaskRow{
		Project:       project,
		CurrentPhase:  raw.CurrentPhase,
		DecisionClass: raw.DecisionClass,
		OpenedAt:      raw.OpenedAt,
		ClosedAt:      raw.ClosedAt,
		Result:        raw.VerifyResult,
	}, true, nil
}

// IndexAll regenerates INDEX.md for every project under projects/<p>/
// AND emits a top-level ~/.bot-hq/INDEX.md cross-project IPAV index.
// Returns the list of project keys whose INDEX.md actually changed +
// whether the top-level INDEX.md changed.
func (c *CL) IndexAll() (changedProjects []string, topChanged bool, err error) {
	projects, err := c.ListProjects()
	if err != nil {
		return nil, false, err
	}
	for _, p := range projects {
		_, changed, err := c.IndexProject(p)
		if err != nil {
			return nil, false, fmt.Errorf("index %s: %w", p, err)
		}
		if changed {
			changedProjects = append(changedProjects, p)
		}
	}
	topChanged, err = c.indexTopLevel(projects)
	if err != nil {
		return changedProjects, false, err
	}
	return changedProjects, topChanged, nil
}

// ListProjects enumerates project keys (subdirs of ~/.bot-hq/projects/
// that have a sibling <project>.yaml file declaring them).
func (c *CL) ListProjects() ([]string, error) {
	dir := filepath.Join(c.root, "projects")
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var out []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		yamlPath := filepath.Join(dir, e.Name()+".yaml")
		if _, err := os.Stat(yamlPath); err != nil {
			continue // unregistered subdir; skip
		}
		out = append(out, e.Name())
	}
	sort.Strings(out)
	return out, nil
}

// indexTopLevel writes ~/.bot-hq/INDEX.md aggregating IPAV tasks across
// all projects. Returns whether the on-disk file changed.
func (c *CL) indexTopLevel(projects []string) (bool, error) {
	var allOpen, allRecent []IPAVTaskRow
	for _, p := range projects {
		projDir := filepath.Join(c.root, "projects", p)
		open, recent, err := c.scanIPAVTasks(p, projDir)
		if err != nil {
			return false, fmt.Errorf("scan %s: %w", p, err)
		}
		allOpen = append(allOpen, open...)
		allRecent = append(allRecent, recent...)
	}

	var b strings.Builder
	fm := IndexFrontmatter{
		GeneratedAt:        time.Now().UTC(),
		Project:            "(all)",
		Generator:          "bot-hq cl-index --all",
		SchemaVersion:      IndexSchemaVersion,
		IPAVTasksOpen:      len(allOpen),
		IPAVTasksClosed30d: len(allRecent),
	}
	if err := writeFrontmatter(&b, fm); err != nil {
		return false, err
	}

	b.WriteString("# Bot-HQ — cross-project IPAV index\n\n")
	b.WriteString("Auto-generated. Regenerate via `bot-hq cl-index --all`.\n\n")
	b.WriteString("Per-project library indexes live at `~/.bot-hq/projects/<project>/INDEX.md`.\n\n")

	b.WriteString("## Registered projects\n\n")
	for _, p := range projects {
		fmt.Fprintf(&b, "- [%s](projects/%s/INDEX.md)\n", p, p)
	}
	b.WriteString("\n")

	b.WriteString("## Active IPAV tasks (all projects)\n\n")
	if len(allOpen) == 0 {
		b.WriteString("(none)\n\n")
	} else {
		b.WriteString("| Project | Task ID | Phase | Decision | Opened |\n")
		b.WriteString("|---------|---------|-------|----------|--------|\n")
		sort.Slice(allOpen, func(i, j int) bool { return allOpen[i].OpenedAt.After(allOpen[j].OpenedAt) })
		for _, t := range allOpen {
			fmt.Fprintf(&b, "| %s | %s | %s | %s | %s |\n", t.Project, t.TaskID, t.CurrentPhase, t.DecisionClass, t.OpenedAt.UTC().Format("2006-01-02"))
		}
		b.WriteString("\n")
	}

	b.WriteString("## Recently closed (last 30 days, all projects)\n\n")
	if len(allRecent) == 0 {
		b.WriteString("(none)\n\n")
	} else {
		b.WriteString("| Project | Task ID | Result | Closed |\n")
		b.WriteString("|---------|---------|--------|--------|\n")
		sort.Slice(allRecent, func(i, j int) bool { return allRecent[i].ClosedAt.After(allRecent[j].ClosedAt) })
		for _, t := range allRecent {
			fmt.Fprintf(&b, "| %s | %s | %s | %s |\n", t.Project, t.TaskID, t.Result, t.ClosedAt.UTC().Format("2006-01-02"))
		}
		b.WriteString("\n")
	}

	indexPath := filepath.Join(c.root, "INDEX.md")
	prev, err := os.ReadFile(indexPath)
	if err != nil && !os.IsNotExist(err) {
		return false, err
	}
	if bytesEqualIgnoringFrontmatterTimestamp(prev, []byte(b.String())) {
		return false, nil
	}
	tmp := indexPath + ".tmp"
	if err := os.WriteFile(tmp, []byte(b.String()), 0o644); err != nil {
		return false, err
	}
	if err := os.Rename(tmp, indexPath); err != nil {
		_ = os.Remove(tmp)
		return false, err
	}
	return true, nil
}

// readSubdir lists immediate-child files of dir (no recursion). Returns
// entries + most-recent ModTime among content entries (README.md and dot-
// files excluded — substrate docs aren't content). Empty/missing dir →
// (nil, zero, nil).
func readSubdir(dir string) ([]os.DirEntry, time.Time, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, time.Time{}, nil
		}
		return nil, time.Time{}, err
	}
	var last time.Time
	for _, e := range entries {
		n := e.Name()
		if n == "README.md" || strings.HasPrefix(n, ".") {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().After(last) {
			last = info.ModTime()
		}
	}
	return entries, last, nil
}

// externalDocsRoot reads projects/<project>.yaml to extract the
// library.external_docs_root field. Returns "" on any failure or missing.
func externalDocsRoot(canonRoot, project string) string {
	yamlPath := filepath.Join(canonRoot, "projects", project+".yaml")
	data, err := os.ReadFile(yamlPath)
	if err != nil {
		return ""
	}
	var raw struct {
		Library struct {
			ExternalDocsRoot string `yaml:"external_docs_root"`
		} `yaml:"library"`
	}
	if err := yaml.Unmarshal(data, &raw); err != nil {
		return ""
	}
	return strings.TrimSpace(raw.Library.ExternalDocsRoot)
}

// expandHome resolves leading ~ to $HOME. Returns the input unchanged
// when no ~ prefix or HOME unresolvable.
func expandHome(p string) string {
	if !strings.HasPrefix(p, "~") {
		return p
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return p
	}
	return filepath.Join(home, strings.TrimPrefix(p, "~"))
}

// formatSize renders bytes as 1.2K / 34M / 5G. Bytes < 1024 render bare.
func formatSize(n int64) string {
	const k = 1024
	if n < k {
		return fmt.Sprintf("%d", n)
	}
	if n < k*k {
		return fmt.Sprintf("%.1fK", float64(n)/k)
	}
	if n < k*k*k {
		return fmt.Sprintf("%.1fM", float64(n)/(k*k))
	}
	return fmt.Sprintf("%.1fG", float64(n)/(k*k*k))
}

// writeFrontmatter emits a YAML frontmatter block (--- delimited) into b.
func writeFrontmatter(b *strings.Builder, fm IndexFrontmatter) error {
	b.WriteString("---\n")
	out, err := yaml.Marshal(fm)
	if err != nil {
		return fmt.Errorf("marshal frontmatter: %w", err)
	}
	b.Write(out)
	b.WriteString("---\n\n")
	return nil
}

// bytesEqualIgnoringFrontmatterTimestamp returns true when prev and next
// match modulo the `generated_at:` line. Used for idempotent writes —
// regeneration without disk-state-change should be a no-op even though
// the timestamp moves every call.
func bytesEqualIgnoringFrontmatterTimestamp(prev, next []byte) bool {
	if len(prev) == 0 {
		return false
	}
	stripTS := func(b []byte) string {
		s := string(b)
		var lines []string
		for _, line := range strings.Split(s, "\n") {
			if strings.HasPrefix(line, "generated_at:") {
				continue
			}
			lines = append(lines, line)
		}
		return strings.Join(lines, "\n")
	}
	return stripTS(prev) == stripTS(next)
}
