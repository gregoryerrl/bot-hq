package webui

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// settingsServer wraps newTestServerWithDB (hub_pivot_handler_test.go)
// for ergonomic db-handle access in settings tests.
func settingsServer(t *testing.T) (*Server, *hub.DB) {
	t.Helper()
	s := newTestServerWithDB(t)
	return s, s.db
}

func callJSON(t *testing.T, s *Server, method, target string, body interface{}) (int, string) {
	t.Helper()
	var rdr io.Reader
	if body != nil {
		raw, _ := json.Marshal(body)
		rdr = bytes.NewReader(raw)
	}
	req := httptest.NewRequest(method, target, rdr)
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	respBody, _ := io.ReadAll(w.Result().Body)
	return w.Code, string(respBody)
}

// ====== List ======

func TestListAgentModelConfigs_seededDefaults(t *testing.T) {

	s, _ := settingsServer(t)

	// hub.OpenDB seeds default rows on schema-create (5 rows per
	// hub/agent_model_configs.go DefaultAgentModelConfigs).
	status, body := callJSON(t, s, "GET", "/api/agent-model-configs", nil)
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	var rows []agentModelConfigDTO
	if err := json.Unmarshal([]byte(body), &rows); err != nil {
		t.Fatalf("unmarshal: %v body=%s", err, body)
	}
	if len(rows) < 1 {
		t.Errorf("expected ≥1 default row, got %d", len(rows))
	}
}

func TestListAgentModelConfigs_methodNotAllowed(t *testing.T) {

	s, _ := settingsServer(t)
	status, _ := callJSON(t, s, "PUT", "/api/agent-model-configs", nil)
	if status != http.StatusMethodNotAllowed {
		t.Errorf("PUT status = %d, want 405", status)
	}
}

// ====== Upsert (POST) ======

func TestUpsertAgentModelConfig_newRow(t *testing.T) {

	s, db := settingsServer(t)

	dto := agentModelConfigDTO{
		AgentID:       "test-coder-xyz",
		Provider:      "anthropic",
		ModelName:     "claude-default",
		AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
		Enabled:       true,
		Notes:         "test row",
	}
	status, body := callJSON(t, s, "POST", "/api/agent-model-configs", dto)
	if status != http.StatusOK {
		t.Fatalf("status = %d, body=%s", status, body)
	}
	var resp agentModelConfigDTO
	if err := json.Unmarshal([]byte(body), &resp); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if resp.AgentID != "test-coder-xyz" {
		t.Errorf("agent_id = %q", resp.AgentID)
	}
	if resp.CreatedAt == "" || resp.UpdatedAt == "" {
		t.Errorf("expected timestamps populated post-upsert: %+v", resp)
	}

	// Verify persisted in DB
	got, err := db.GetAgentModelConfig("test-coder-xyz")
	if err != nil {
		t.Fatalf("GetAgentModelConfig: %v", err)
	}
	if got.Provider != "anthropic" {
		t.Errorf("persisted provider = %q", got.Provider)
	}
}

func TestUpsertAgentModelConfig_updateExisting(t *testing.T) {

	s, db := settingsServer(t)

	// Pre-seed via direct DB
	_ = db.SetAgentModelConfig(&hub.AgentModelConfig{
		AgentID:       "myagent",
		Provider:      "anthropic",
		ModelName:     "claude-default",
		AuthSecretRef: "oauth:OLD",
		Enabled:       true,
	})
	dto := agentModelConfigDTO{
		AgentID:       "myagent",
		Provider:      "anthropic",
		ModelName:     "claude-default",
		AuthSecretRef: "oauth:NEW", // changed
		Enabled:       true,
	}
	status, _ := callJSON(t, s, "POST", "/api/agent-model-configs", dto)
	if status != http.StatusOK {
		t.Errorf("update status = %d", status)
	}
	got, _ := db.GetAgentModelConfig("myagent")
	if got.AuthSecretRef != "oauth:NEW" {
		t.Errorf("update did not persist: AuthSecretRef = %q", got.AuthSecretRef)
	}
}

func TestUpsertAgentModelConfig_validationErrors(t *testing.T) {

	s, _ := settingsServer(t)

	cases := []struct {
		name string
		dto  agentModelConfigDTO
	}{
		{"empty agent_id", agentModelConfigDTO{Provider: "x", ModelName: "y", AuthSecretRef: "z"}},
		{"empty provider", agentModelConfigDTO{AgentID: "a", ModelName: "y", AuthSecretRef: "z"}},
		{"empty model_name", agentModelConfigDTO{AgentID: "a", Provider: "x", AuthSecretRef: "z"}},
		{"empty auth_secret_ref", agentModelConfigDTO{AgentID: "a", Provider: "x", ModelName: "y"}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			status, body := callJSON(t, s, "POST", "/api/agent-model-configs", c.dto)
			if status != http.StatusBadRequest {
				t.Errorf("status = %d, want 400 (body=%s)", status, body)
			}
		})
	}
}

func TestUpsertAgentModelConfig_invalidJSON(t *testing.T) {

	s, _ := settingsServer(t)
	req := httptest.NewRequest("POST", "/api/agent-model-configs", bytes.NewReader([]byte("not-json{{{")))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.httpServer.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusBadRequest {
		t.Errorf("invalid-JSON status = %d", w.Code)
	}
}

// ====== Delete ======

func TestDeleteAgentModelConfig_existingRow(t *testing.T) {

	s, db := settingsServer(t)

	_ = db.SetAgentModelConfig(&hub.AgentModelConfig{
		AgentID:       "delme",
		Provider:      "anthropic",
		ModelName:     "claude-default",
		AuthSecretRef: "oauth:X",
		Enabled:       true,
	})
	status, _ := callJSON(t, s, "DELETE", "/api/agent-model-configs/delme", nil)
	if status != http.StatusNoContent {
		t.Errorf("delete status = %d, want 204", status)
	}
	if _, err := db.GetAgentModelConfig("delme"); err == nil {
		t.Error("row still exists post-delete")
	}
}

func TestDeleteAgentModelConfig_idempotentMissingRow(t *testing.T) {

	s, _ := settingsServer(t)
	// hub.DeleteAgentModelConfig is idempotent — returns 204 even when no row.
	status, _ := callJSON(t, s, "DELETE", "/api/agent-model-configs/nonexistent-xyz", nil)
	if status != http.StatusNoContent {
		t.Errorf("delete missing status = %d, want 204 (idempotent)", status)
	}
}

func TestDeleteAgentModelConfig_emptyIDRejected(t *testing.T) {

	s, _ := settingsServer(t)
	status, _ := callJSON(t, s, "DELETE", "/api/agent-model-configs/", nil)
	if status != http.StatusBadRequest {
		t.Errorf("empty-id delete status = %d, want 400", status)
	}
}

func TestDeleteAgentModelConfig_methodNotAllowed(t *testing.T) {

	s, _ := settingsServer(t)
	status, _ := callJSON(t, s, "GET", "/api/agent-model-configs/some-agent", nil)
	if status != http.StatusMethodNotAllowed {
		t.Errorf("GET on by-id status = %d, want 405", status)
	}
}

// ====== DTO marshaling ======

func TestToDTOFromDTORoundTrip(t *testing.T) {
	cfg := &hub.AgentModelConfig{
		AgentID:       "x",
		Provider:      "deepseek",
		ModelName:     "deepseek-v4-pro",
		BaseURL:       "https://api.deepseek.com/anthropic",
		AuthSecretRef: "env:DEEPSEEK_API_KEY",
		Enabled:       true,
		Notes:         "test",
	}
	d := toDTO(cfg)
	back := fromDTO(d)
	if back.AgentID != cfg.AgentID || back.Provider != cfg.Provider || back.BaseURL != cfg.BaseURL {
		t.Errorf("roundtrip lost data: %+v vs %+v", back, cfg)
	}
}
