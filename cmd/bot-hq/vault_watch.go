// Phase T T-10 cycle-3: vault watcher wire-up for the bot-hq daemon.
//
// Discovers `file:<path>#<key>` entries in agent_model_configs and watches
// each path for mtime advance. On change, emits a hub MsgUpdate so the
// user gets feedback without having to "signal me when done" via hub
// channel — eliminates the paste-to-hub temptation class.

package main

import (
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/vault"
)

// startVaultWatcher discovers file: scheme paths from enabled agent_model_configs
// and starts a Watcher over them. The change callback inserts a hub MsgUpdate
// naming the agent + path. Returns the running Watcher so the caller can
// defer Stop(); returns nil when no file: scheme paths are present.
func startVaultWatcher(h *hub.Hub) *vault.Watcher {
	configs, err := h.DB.ListAgentModelConfigs(true)
	if err != nil {
		log.Printf("[autostart] vault-watcher: list configs failed: %v", err)
		return nil
	}

	pathToAgent := make(map[string]string)
	var paths []string
	for _, cfg := range configs {
		if !strings.HasPrefix(cfg.AuthSecretRef, "file:") {
			continue
		}
		path, _, err := parseFileScheme(cfg.AuthSecretRef)
		if err != nil {
			log.Printf("[autostart] vault-watcher: skip %s: %v", cfg.AgentID, err)
			continue
		}
		// Deduplicate when multiple agents share a vault file.
		if _, ok := pathToAgent[path]; !ok {
			paths = append(paths, path)
		}
		pathToAgent[path] = cfg.AgentID
	}

	if len(paths) == 0 {
		log.Printf("[autostart] vault-watcher: no file: scheme entries; skipping")
		return nil
	}

	w := vault.NewWatcher(paths, 30*time.Second, func(path string) {
		agentID := pathToAgent[path]
		log.Printf("[vault-watcher] rotation detected for %s at %s", agentID, path)
		_, err := h.DB.InsertMessage(protocol.Message{
			FromAgent: "system",
			Type:      protocol.MsgUpdate,
			Content: fmt.Sprintf(
				"vault-watcher|rotation-detected|agent=%s|path=%s|new-secret-takes-effect-on-next-agent-spawn(kill+respawn-OR-daemon-restart)",
				agentID, path,
			),
		})
		if err != nil {
			log.Printf("[vault-watcher] insert message failed: %v", err)
		}
	})
	w.Start()
	log.Printf("[autostart] vault-watcher OK (%d path(s) watched)", len(paths))
	return w
}

// parseFileScheme parses a `file:<path>#<key>` reference-pointer and returns
// the on-disk path (with ~ expanded) and the key name. Mirrors the resolver
// at internal/agentconfig/resolver.go:resolveFileSecret to keep the watch
// surface aligned with the resolution surface.
func parseFileScheme(ref string) (path, key string, err error) {
	const prefix = "file:"
	if !strings.HasPrefix(ref, prefix) {
		return "", "", errors.New("not a file: scheme ref")
	}
	body := strings.TrimPrefix(ref, prefix)
	rawPath, k, found := strings.Cut(body, "#")
	if !found {
		return "", "", fmt.Errorf("file: ref missing #KEY suffix: %s", ref)
	}
	if rawPath == "" {
		return "", "", errors.New("empty file path in ref")
	}
	if k == "" {
		return "", "", errors.New("empty key in ref")
	}
	if strings.HasPrefix(rawPath, "~/") {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", "", fmt.Errorf("home dir: %w", err)
		}
		rawPath = filepath.Join(home, rawPath[2:])
	}
	return rawPath, k, nil
}
