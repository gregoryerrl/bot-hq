package cl

import (
	"path/filepath"
	"strings"
)

// HiddenPathsConfig categorizes the canonical-store hide rules for tree-walk
// responses. Different categories apply at different depths.
type HiddenPathsConfig struct {
	// TopLevelBasenames are filename matches hidden at depth==0 (and
	// always-hidden if their extension is also in AnyDepthExtensions).
	TopLevelBasenames map[string]struct{}

	// TopLevelOnlyBasenames are hidden ONLY at depth==0; the same basename
	// inside projects/<p>/ or deeper is allowed. Per plan-doc §2.1:
	// voice-mirror-log.md is CL-runtime at the root but a per-project EOD
	// surface inside projects/<p>/.
	TopLevelOnlyBasenames map[string]struct{}

	// AnyDepthBasenames are hidden regardless of depth (runtime state files
	// that may appear anywhere).
	AnyDepthBasenames map[string]struct{}

	// AnyDepthExtensions are file-extensions hidden regardless of depth
	// (sqlite/log/jsonl runtime + source code).
	AnyDepthExtensions map[string]struct{}

	// TopLevelDirs are directory basenames that hide their entire subtree
	// when they appear at depth==0. Contents under these dirs are surfaced
	// via dedicated routes (gates → Rules destination; sessions →
	// /api/sessions) and not by the raw tree-walker.
	TopLevelDirs map[string]struct{}

	// AgentDirs are top-level agent directories. Tree-walker ENTERS these
	// (depth==0 dir not itself hidden), but contents are deny-by-default
	// — only AgentSubpathAllowlist basenames surface.
	AgentDirs map[string]struct{}

	// AgentSubpathAllowlist names the basenames inside an AgentDir that
	// ARE allowed to surface (override the deny-by-default).
	AgentSubpathAllowlist map[string]struct{}
}

// HiddenPaths is the canonical hide-list for CL tree responses. Source-of-
// truth migrated from internal/webui/filesystem.go (canonicalSkipList +
// hideListBasenames + hideListExtensions + hideListTopDirs + canonicalSkipAgentDirs
// + allowlistedAgentSubpaths) per phase-r-followup-cl-uniformity Plan §3 S4.
//
// Consumed by the webui tree-walker, INDEX-regen scope detection, and
// cross-project query filter chain. Single source of truth across the
// daemon ensures no surface diverges from another.
var HiddenPaths = HiddenPathsConfig{
	TopLevelBasenames: map[string]struct{}{
		"hub.db":         {},
		"hub.db-shm":     {},
		"hub.db-wal":     {},
		"bot-hq.db":      {},
		"webui-index.db": {},
		"live.log":       {},
		"debug.log":      {},
	},
	TopLevelOnlyBasenames: map[string]struct{}{
		"voice-mirror-log.md": {},
	},
	AnyDepthBasenames: map[string]struct{}{
		"last_state.json": {},
	},
	AnyDepthExtensions: map[string]struct{}{
		".db":     {},
		".db-wal": {},
		".db-shm": {},
		".log":    {},
		".jsonl":  {},
		".ts":     {},
		".js":     {},
	},
	TopLevelDirs: map[string]struct{}{
		"diag":      {},
		"sentinels": {},
		"bridge":    {},
		"plugins":   {},
		"gates":     {},
		"sessions":  {},
	},
	AgentDirs: map[string]struct{}{
		"brian": {},
		"rain":  {},
		"emma":  {},
		"clive": {},
	},
	AgentSubpathAllowlist: map[string]struct{}{
		"discipline-anchors.md": {},
	},
}

// IsHidden returns true if a canonical-store-relative path should be
// excluded from tree responses. relPath uses forward-slash separators,
// no leading slash, and is the path RELATIVE to the canonical-store root
// (e.g., "hub.db", "projects/bot-hq/INDEX.md", "brian/discipline-anchors.md").
//
// isDir distinguishes file vs directory; matters for TopLevelDirs (which
// only block when applied to directories at depth 0).
func IsHidden(relPath string, isDir bool) bool {
	relPath = filepath.ToSlash(relPath)
	relPath = strings.TrimPrefix(relPath, "./")
	if relPath == "" || relPath == "." {
		return false
	}
	parts := strings.Split(relPath, "/")
	for _, p := range parts {
		if p != "." && strings.HasPrefix(p, ".") {
			return true
		}
	}
	depth := len(parts) - 1
	base := parts[len(parts)-1]

	if depth == 0 {
		if _, ok := HiddenPaths.TopLevelBasenames[base]; ok {
			return true
		}
		if _, ok := HiddenPaths.TopLevelOnlyBasenames[base]; ok {
			return true
		}
		if isDir {
			if _, ok := HiddenPaths.TopLevelDirs[base]; ok {
				return true
			}
		}
	}

	if _, ok := HiddenPaths.TopLevelDirs[parts[0]]; ok {
		return true
	}

	if _, isAgent := HiddenPaths.AgentDirs[parts[0]]; isAgent && depth >= 1 {
		if _, ok := HiddenPaths.AgentSubpathAllowlist[base]; !ok {
			return true
		}
	}

	if _, ok := HiddenPaths.AnyDepthBasenames[base]; ok {
		return true
	}
	if ext := filepath.Ext(base); ext != "" {
		if _, ok := HiddenPaths.AnyDepthExtensions[ext]; ok {
			return true
		}
	}

	return false
}
