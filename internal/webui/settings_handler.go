// Package webui — settings_handler.go: T-1.3 web UI settings tab backend.
//
// Exposes agent_model_configs CRUD via HTTP for the frontend settings tab:
//
//	GET    /api/agent-model-configs        — list all configs
//	POST   /api/agent-model-configs        — upsert one config
//	DELETE /api/agent-model-configs/{id}   — delete by agent_id
//
// Authentication: loopback-only per existing webui scope. No additional
// auth required (single-user local development).

package webui

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// agentModelConfigDTO mirrors hub.AgentModelConfig for JSON wire-format.
// Time fields rendered as RFC3339; enabled as bool.
type agentModelConfigDTO struct {
	AgentID       string `json:"agent_id"`
	Provider      string `json:"provider"`
	ModelName     string `json:"model_name"`
	BaseURL       string `json:"base_url,omitempty"`
	AuthSecretRef string `json:"auth_secret_ref"`
	Enabled       bool   `json:"enabled"`
	Notes         string `json:"notes,omitempty"`
	CreatedAt     string `json:"created_at,omitempty"`
	UpdatedAt     string `json:"updated_at,omitempty"`
}

func toDTO(c *hub.AgentModelConfig) agentModelConfigDTO {
	return agentModelConfigDTO{
		AgentID:       c.AgentID,
		Provider:      c.Provider,
		ModelName:     c.ModelName,
		BaseURL:       c.BaseURL,
		AuthSecretRef: c.AuthSecretRef,
		Enabled:       c.Enabled,
		Notes:         c.Notes,
		CreatedAt:     c.CreatedAt.Format("2006-01-02T15:04:05Z07:00"),
		UpdatedAt:     c.UpdatedAt.Format("2006-01-02T15:04:05Z07:00"),
	}
}

func fromDTO(d agentModelConfigDTO) *hub.AgentModelConfig {
	return &hub.AgentModelConfig{
		AgentID:       d.AgentID,
		Provider:      d.Provider,
		ModelName:     d.ModelName,
		BaseURL:       d.BaseURL,
		AuthSecretRef: d.AuthSecretRef,
		Enabled:       d.Enabled,
		Notes:         d.Notes,
	}
}

// handleAgentModelConfigs dispatches GET/POST on /api/agent-model-configs.
func (s *Server) handleAgentModelConfigs(w http.ResponseWriter, r *http.Request) {
	if s.db == nil {
		http.Error(w, "database not configured", http.StatusServiceUnavailable)
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.listAgentModelConfigs(w, r)
	case http.MethodPost:
		s.upsertAgentModelConfig(w, r)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

// handleAgentModelConfigByID dispatches DELETE on /api/agent-model-configs/{id}.
func (s *Server) handleAgentModelConfigByID(w http.ResponseWriter, r *http.Request) {
	if s.db == nil {
		http.Error(w, "database not configured", http.StatusServiceUnavailable)
		return
	}
	if r.Method != http.MethodDelete {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	agentID := strings.TrimPrefix(r.URL.Path, "/api/agent-model-configs/")
	if agentID == "" {
		http.Error(w, "agent_id required in path", http.StatusBadRequest)
		return
	}
	if err := s.db.DeleteAgentModelConfig(agentID); err != nil {
		http.Error(w, fmt.Sprintf("delete: %v", err), http.StatusInternalServerError)
		return
	}
	// hub.DeleteAgentModelConfig is idempotent (no error on missing row);
	// 204 returned uniformly per REST idempotency semantics.
	w.WriteHeader(http.StatusNoContent)
}

func (s *Server) listAgentModelConfigs(w http.ResponseWriter, r *http.Request) {
	enabledOnly := r.URL.Query().Get("enabled_only") == "true"
	rows, err := s.db.ListAgentModelConfigs(enabledOnly)
	if err != nil {
		http.Error(w, fmt.Sprintf("list: %v", err), http.StatusInternalServerError)
		return
	}
	out := make([]agentModelConfigDTO, 0, len(rows))
	for _, c := range rows {
		out = append(out, toDTO(c))
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(out)
}

func (s *Server) upsertAgentModelConfig(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		http.Error(w, "read body", http.StatusBadRequest)
		return
	}
	var dto agentModelConfigDTO
	if err := json.Unmarshal(body, &dto); err != nil {
		http.Error(w, fmt.Sprintf("invalid JSON: %v", err), http.StatusBadRequest)
		return
	}
	if err := validateConfigDTO(dto); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	cfg := fromDTO(dto)
	if err := s.db.SetAgentModelConfig(cfg); err != nil {
		http.Error(w, fmt.Sprintf("upsert: %v", err), http.StatusInternalServerError)
		return
	}
	// Reload to capture CreatedAt/UpdatedAt timestamps.
	saved, err := s.db.GetAgentModelConfig(cfg.AgentID)
	if err != nil {
		http.Error(w, fmt.Sprintf("reload: %v", err), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	_ = json.NewEncoder(w).Encode(toDTO(saved))
}

func validateConfigDTO(d agentModelConfigDTO) error {
	if d.AgentID == "" {
		return errors.New("agent_id is required")
	}
	if d.Provider == "" {
		return errors.New("provider is required")
	}
	if d.ModelName == "" {
		return errors.New("model_name is required")
	}
	if d.AuthSecretRef == "" {
		return errors.New("auth_secret_ref is required")
	}
	return nil
}
