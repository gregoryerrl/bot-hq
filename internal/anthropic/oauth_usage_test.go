package anthropic

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

// fakeCreds is a deterministic CredentialSource for tests.
type fakeCreds struct {
	creds Credentials
	err   error
}

func (f fakeCreds) Get(context.Context) (Credentials, error) {
	return f.creds, f.err
}

// stubServer returns an httptest.Server that serves the given JSON body at
// /api/oauth/usage with the given status code. Captures the Authorization
// header so tests can assert the bearer wiring.
func stubServer(t *testing.T, status int, body string) (*httptest.Server, *string) {
	t.Helper()
	var lastAuth string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/oauth/usage" {
			t.Errorf("unexpected path %q", r.URL.Path)
		}
		lastAuth = r.Header.Get("Authorization")
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(status)
		w.Write([]byte(body))
	}))
	t.Cleanup(srv.Close)
	return srv, &lastAuth
}

// validToken is a non-expired credential to drive happy-path fetches.
func validToken() Credentials {
	return Credentials{AccessToken: "sk-test", ExpiresAt: time.Now().Add(1 * time.Hour)}
}

// TestFetchAllFourWindowsMaxUtilDetected locks the canonical happy path:
// API returns four windows; client picks the max + names the binding window.
func TestFetchAllFourWindowsMaxUtilDetected(t *testing.T) {
	body := `{
		"five_hour":        {"utilization": 0.20, "resets_at": "2026-04-28T12:00:00Z"},
		"seven_day":        {"utilization": 0.40, "resets_at": "2026-05-04T00:00:00Z"},
		"seven_day_sonnet": {"utilization": 0.10, "resets_at": "2026-05-04T00:00:00Z"},
		"seven_day_opus":   {"utilization": 0.95, "resets_at": "2026-05-04T00:00:00Z"}
	}`
	srv, _ := stubServer(t, http.StatusOK, body)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})

	maxUtil, win, per, err := c.Fetch(context.Background())
	if err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if win != WindowSevenDayOpus {
		t.Errorf("maxWindow = %q, want %q", win, WindowSevenDayOpus)
	}
	if maxUtil != 0.95 {
		t.Errorf("maxUtil = %v, want 0.95", maxUtil)
	}
	if len(per) != 4 {
		t.Errorf("perWindow size = %d, want 4", len(per))
	}
	if per[WindowFiveHour].Utilization != 0.20 {
		t.Errorf("five_hour utilization = %v, want 0.20", per[WindowFiveHour].Utilization)
	}
}

// TestFetchOptsMissingOpusWindow locks the lower-tier shape: no opus
// window present, max correctly resolved across the remaining three.
func TestFetchOptsMissingOpusWindow(t *testing.T) {
	body := `{
		"five_hour":        {"utilization": 0.92, "resets_at": "2026-04-28T12:00:00Z"},
		"seven_day":        {"utilization": 0.40, "resets_at": "2026-05-04T00:00:00Z"},
		"seven_day_sonnet": {"utilization": 0.10, "resets_at": "2026-05-04T00:00:00Z"}
	}`
	srv, _ := stubServer(t, http.StatusOK, body)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})

	maxUtil, win, per, err := c.Fetch(context.Background())
	if err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if win != WindowFiveHour {
		t.Errorf("maxWindow = %q, want five_hour", win)
	}
	if maxUtil != 0.92 {
		t.Errorf("maxUtil = %v, want 0.92", maxUtil)
	}
	if _, ok := per[WindowSevenDayOpus]; ok {
		t.Errorf("perWindow should not contain opus key when absent from response")
	}
}

// TestFetchPassesBearerAuth locks the Authorization header wiring. A
// silent missing-bearer regression would surface as 401 in production but
// look like a transient auth failure to operators.
func TestFetchPassesBearerAuth(t *testing.T) {
	body := `{"five_hour":{"utilization":0.5,"resets_at":"2026-04-28T12:00:00Z"}}`
	srv, lastAuth := stubServer(t, http.StatusOK, body)
	c := NewUsageClient(srv.URL, fakeCreds{creds: Credentials{AccessToken: "sk-XYZ", ExpiresAt: time.Now().Add(time.Hour)}})

	if _, _, _, err := c.Fetch(context.Background()); err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if got, want := *lastAuth, "Bearer sk-XYZ"; got != want {
		t.Errorf("Authorization header = %q, want %q", got, want)
	}
}

// TestFetchAuthFailureSurfacesAsError locks the 401 path. Caller (Emma's
// plan_usage producer) treats this identically to 5xx — backoff applies.
func TestFetchAuthFailureSurfacesAsError(t *testing.T) {
	srv, _ := stubServer(t, http.StatusUnauthorized, `{"error":"unauth"}`)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})
	_, _, _, err := c.Fetch(context.Background())
	if err == nil {
		t.Fatal("expected error for 401, got nil")
	}
}

// TestFetch5xxSurfacesAsError locks the upstream-error path.
func TestFetch5xxSurfacesAsError(t *testing.T) {
	srv, _ := stubServer(t, http.StatusBadGateway, ``)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})
	_, _, _, err := c.Fetch(context.Background())
	if err == nil {
		t.Fatal("expected error for 502, got nil")
	}
}

// TestFetchNearExpirySkipsAPI locks the 5min runway: a token that expires
// within keychainExpirySkew returns ErrTokenExpired BEFORE the HTTP call,
// so a stub server that fails the test if reached confirms the skip.
func TestFetchNearExpirySkipsAPI(t *testing.T) {
	called := false
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		called = true
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	c := NewUsageClient(srv.URL, fakeCreds{creds: Credentials{
		AccessToken: "sk-test",
		ExpiresAt:   time.Now().Add(2 * time.Minute), // < keychainExpirySkew
	}})
	_, _, _, err := c.Fetch(context.Background())
	if !errors.Is(err, ErrTokenExpired) {
		t.Errorf("expected ErrTokenExpired, got %v", err)
	}
	if called {
		t.Errorf("near-expiry token must not trigger HTTP call")
	}
}

// TestFetchExpiredTokenWithCustomNow stages a deterministic expiry by
// overriding the clock; locks the SetNow hook + the boundary semantic
// (now+skew > expiresAt → ErrTokenExpired).
func TestFetchExpiredTokenWithCustomNow(t *testing.T) {
	srv, _ := stubServer(t, http.StatusOK, `{"five_hour":{"utilization":0.5,"resets_at":"2026-04-28T12:00:00Z"}}`)
	expiry := time.Date(2026, 4, 28, 12, 0, 0, 0, time.UTC)

	c := NewUsageClient(srv.URL, fakeCreds{creds: Credentials{
		AccessToken: "sk-test", ExpiresAt: expiry,
	}})
	// Now exactly at the skew boundary — strictly inside the runway.
	c.SetNow(func() time.Time { return expiry.Add(-keychainExpirySkew + time.Second) })

	if _, _, _, err := c.Fetch(context.Background()); !errors.Is(err, ErrTokenExpired) {
		t.Errorf("inside skew window: got %v, want ErrTokenExpired", err)
	}
}

// TestFetchValidTokenWithCustomNow is the negative-control twin: just
// outside the skew window, Fetch proceeds normally.
func TestFetchValidTokenWithCustomNow(t *testing.T) {
	srv, _ := stubServer(t, http.StatusOK, `{"five_hour":{"utilization":0.5,"resets_at":"2026-04-28T12:00:00Z"}}`)
	expiry := time.Date(2026, 4, 28, 12, 0, 0, 0, time.UTC)

	c := NewUsageClient(srv.URL, fakeCreds{creds: Credentials{
		AccessToken: "sk-test", ExpiresAt: expiry,
	}})
	c.SetNow(func() time.Time { return expiry.Add(-2 * keychainExpirySkew) })

	if _, _, _, err := c.Fetch(context.Background()); err != nil {
		t.Errorf("outside skew window: Fetch should succeed, got %v", err)
	}
}

// TestFetchMalformedJSON surfaces as decode error — caller backoff applies.
func TestFetchMalformedJSON(t *testing.T) {
	srv, _ := stubServer(t, http.StatusOK, `not-json`)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})
	if _, _, _, err := c.Fetch(context.Background()); err == nil {
		t.Errorf("malformed JSON must surface as error")
	}
}

// TestFetchEmptyResponseSurfacesError locks the no-windows-present case:
// API returns valid JSON with zero recognized windows. Returning a
// generic-success would mislead the caller into PlanUsagePct=0%.
func TestFetchEmptyResponseSurfacesError(t *testing.T) {
	srv, _ := stubServer(t, http.StatusOK, `{}`)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})
	if _, _, _, err := c.Fetch(context.Background()); err == nil {
		t.Errorf("empty windows must surface as error")
	}
}

// TestParseKeychainBlobValid locks the JSON shape from cli.js 2.1.73:
// `claudeAiOauth.accessToken` + `claudeAiOauth.expiresAt` (unix ms).
func TestParseKeychainBlobValid(t *testing.T) {
	expiresAt := time.Now().Add(1 * time.Hour).UnixMilli()
	blob := map[string]any{
		"claudeAiOauth": map[string]any{
			"accessToken": "sk-keychain",
			"expiresAt":   expiresAt,
		},
	}
	raw, _ := json.Marshal(blob)
	raw = append(raw, '\n') // simulate the CLI trailing newline

	creds, err := parseKeychainBlob(raw)
	if err != nil {
		t.Fatalf("parseKeychainBlob: %v", err)
	}
	if creds.AccessToken != "sk-keychain" {
		t.Errorf("accessToken = %q, want sk-keychain", creds.AccessToken)
	}
	if creds.ExpiresAt.UnixMilli() != expiresAt {
		t.Errorf("expiresAt = %v, want %v", creds.ExpiresAt.UnixMilli(), expiresAt)
	}
}

// TestParseKeychainBlobMissingToken locks the contract: a valid-JSON blob
// without claudeAiOauth.accessToken surfaces as error rather than
// returning an empty bearer that would 401 downstream.
func TestParseKeychainBlobMissingToken(t *testing.T) {
	if _, err := parseKeychainBlob([]byte(`{"claudeAiOauth":{}}`)); err == nil {
		t.Errorf("missing accessToken must surface as error")
	}
}

// TestParseKeychainBlobMalformed locks decode-error fallthrough.
func TestParseKeychainBlobMalformed(t *testing.T) {
	if _, err := parseKeychainBlob([]byte(`not-json`)); err == nil {
		t.Errorf("malformed JSON must surface as error")
	}
}
