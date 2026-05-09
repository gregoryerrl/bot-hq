// Package cl — crossref.go: T-1.8 Cross-reference graph for CL artifacts.
//
// Builds a graph (artifact → cross-referenced files) by scanning every CL
// artifact's content for file-path + line-anchor citations via the
// citeanchor extractor. Used for impact-analysis (which artifacts cite a
// given file?) + drift-detection (broken cross-refs surface from cite-
// anchor validation in T-1.9).

package cl

import (
	"context"
	"fmt"
	"sort"

	"github.com/gregoryerrl/bot-hq/internal/citeanchor"
)

// CrossRefEdge is one directed edge in the cross-ref graph: from artifact
// (Path) to a cited file (Target). Anchor preserves the original citation
// for diagnostic display.
type CrossRefEdge struct {
	FromPath string             // artifact that contains the cite
	Target   string             // cited file path (resolved if absolute)
	Anchor   citeanchor.CiteAnchor
}

// CrossRefGraph is the result of BuildCrossRefGraph: a forward-index
// (FromPath → []edges) and a reverse-index (Target → []edges).
type CrossRefGraph struct {
	Forward map[string][]CrossRefEdge
	Reverse map[string][]CrossRefEdge
}

// BuildCrossRefGraph walks the CL + extracts file-path + line-anchor
// citations from each artifact's content. Returns the populated graph.
// Walks the same artifacts as Walk(); skips runtime-ephemera.
func (c *CL) BuildCrossRefGraph(ctx context.Context) (*CrossRefGraph, error) {
	graph := &CrossRefGraph{
		Forward: make(map[string][]CrossRefEdge),
		Reverse: make(map[string][]CrossRefEdge),
	}
	err := c.Walk(func(a *Artifact) error {
		// Read content if not already loaded
		full, err := c.Read(a.Path)
		if err != nil {
			return nil // skip read-errors (don't fail entire graph build)
		}
		anchors := citeanchor.ExtractAnchors(string(full.Content), full.Path)
		for _, anchor := range anchors {
			if anchor.Class != "file-path" && anchor.Class != "line-anchor" {
				continue
			}
			edge := CrossRefEdge{
				FromPath: full.Path,
				Target:   anchor.Value,
				Anchor:   anchor,
			}
			graph.Forward[full.Path] = append(graph.Forward[full.Path], edge)
			graph.Reverse[anchor.Value] = append(graph.Reverse[anchor.Value], edge)
		}
		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("walk: %w", err)
	}
	return graph, nil
}

// CitedBy returns all CL artifacts that cite the given target file.
// Useful for impact-analysis: "what scope-lock-docs reference this file?"
func (g *CrossRefGraph) CitedBy(target string) []CrossRefEdge {
	edges := g.Reverse[target]
	out := make([]CrossRefEdge, len(edges))
	copy(out, edges)
	sort.Slice(out, func(i, j int) bool { return out[i].FromPath < out[j].FromPath })
	return out
}

// Cites returns all targets cited by the given source artifact.
func (g *CrossRefGraph) Cites(fromPath string) []CrossRefEdge {
	edges := g.Forward[fromPath]
	out := make([]CrossRefEdge, len(edges))
	copy(out, edges)
	sort.Slice(out, func(i, j int) bool { return out[i].Target < out[j].Target })
	return out
}

// AllSources returns all source-paths that have at least one outgoing cite.
// Sorted alphabetically.
func (g *CrossRefGraph) AllSources() []string {
	out := make([]string, 0, len(g.Forward))
	for k := range g.Forward {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

// AllTargets returns all target-paths that have at least one incoming cite.
// Sorted alphabetically.
func (g *CrossRefGraph) AllTargets() []string {
	out := make([]string, 0, len(g.Reverse))
	for k := range g.Reverse {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

// EdgeCount returns the total number of edges in the graph (sum across
// all forward-edges).
func (g *CrossRefGraph) EdgeCount() int {
	n := 0
	for _, edges := range g.Forward {
		n += len(edges)
	}
	return n
}
