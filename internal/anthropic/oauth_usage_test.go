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
// + anthropic-beta + User-Agent headers so tests can assert the auth-gate
// wiring (cli.js 2.1.73 sends all three; missing anthropic-beta returns 401
// "OAuth authentication is currently not supported" against production).
func stubServer(t *testing.T, status int, body string) (*httptest.Server, *capturedHeaders) {
	t.Helper()
	captured := &capturedHeaders{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/oauth/usage" {
			t.Errorf("unexpected path %q", r.URL.Path)
		}
		captured.Authorization = r.Header.Get("Authorization")
		captured.AnthropicBeta = r.Header.Get("anthropic-beta")
		captured.UserAgent = r.Header.Get("User-Agent")
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(status)
		w.Write([]byte(body))
	}))
	t.Cleanup(srv.Close)
	return srv, captured
}

type capturedHeaders struct {
	Authorization string
	AnthropicBeta string
	UserAgent     string
}

// validToken is a non-expired credential to drive happy-path fetches.
func validToken() Credentials {
	return Credentials{AccessToken: "sk-test", ExpiresAt: time.Now().Add(1 * time.Hour)}
}

// TestFetchAllFourWindowsMaxUtilDetected locks the canonical happy path:
// API returns four windows; client picks the max + names the binding window.
func TestFetchAllFourWindowsMaxUtilDetected(t *testing.T) {
	// Wire format is 0-100 (verified against live /api/oauth/usage); Fetch
	// normalizes to 0-1 before return so downstream threshold logic is
	// unit-stable. Assertions below check the normalized public surface.
	body := `{
		"five_hour":        {"utilization": 20.0, "resets_at": "2026-04-28T12:00:00Z"},
		"seven_day":        {"utilization": 40.0, "resets_at": "2026-05-04T00:00:00Z"},
		"seven_day_sonnet": {"utilization": 10.0, "resets_at": "2026-05-04T00:00:00Z"},
		"seven_day_opus":   {"utilization": 95.0, "resets_at": "2026-05-04T00:00:00Z"}
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
		"five_hour":        {"utilization": 92.0, "resets_at": "2026-04-28T12:00:00Z"},
		"seven_day":        {"utilization": 40.0, "resets_at": "2026-05-04T00:00:00Z"},
		"seven_day_sonnet": {"utilization": 10.0, "resets_at": "2026-05-04T00:00:00Z"}
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
	body := `{"five_hour":{"utilization":50.0,"resets_at":"2026-04-28T12:00:00Z"}}`
	srv, hdrs := stubServer(t, http.StatusOK, body)
	c := NewUsageClient(srv.URL, fakeCreds{creds: Credentials{AccessToken: "sk-XYZ", ExpiresAt: time.Now().Add(time.Hour)}})

	if _, _, _, err := c.Fetch(context.Background()); err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if got, want := hdrs.Authorization, "Bearer sk-XYZ"; got != want {
		t.Errorf("Authorization header = %q, want %q", got, want)
	}
}

// TestFetchPassesAnthropicBetaAndUserAgent locks the OAuth-on-/usage gate
// wiring. Production-grounded ratchet: without anthropic-beta the API
// returns 401 "OAuth authentication is currently not supported" — silent
// regression here would re-introduce the slice-5 H-32 producer-blank bug.
// User-Agent parity with cli.js 2.1.73 keeps server-side request fingerprint
// consistent with the published client.
func TestFetchPassesAnthropicBetaAndUserAgent(t *testing.T) {
	body := `{"five_hour":{"utilization":50.0,"resets_at":"2026-04-28T12:00:00Z"}}`
	srv, hdrs := stubServer(t, http.StatusOK, body)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})

	if _, _, _, err := c.Fetch(context.Background()); err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if got, want := hdrs.AnthropicBeta, "oauth-2025-04-20"; got != want {
		t.Errorf("anthropic-beta header = %q, want %q", got, want)
	}
	if hdrs.UserAgent == "" {
		t.Errorf("User-Agent header missing; want claude-code/<version>")
	}
}

// TestFetchNormalizesWireToZeroOne locks the unit-conversion contract:
// API returns utilization 0-100; Fetch divides by 100 so all downstream
// threshold comparisons stay in the 0-1 convention shared with
// context_cap.go (planUsageThreshold=0.95). Regression here would make the
// strip render 100% on every poll once auth-gate is satisfied.
func TestFetchNormalizesWireToZeroOne(t *testing.T) {
	body := `{"five_hour":{"utilization":91.0,"resets_at":"2026-04-28T12:00:00Z"}}`
	srv, _ := stubServer(t, http.StatusOK, body)
	c := NewUsageClient(srv.URL, fakeCreds{creds: validToken()})

	maxUtil, _, per, err := c.Fetch(context.Background())
	if err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if maxUtil != 0.91 {
		t.Errorf("maxUtil = %v, want 0.91 (normalized from wire 91.0)", maxUtil)
	}
	if per[WindowFiveHour].Utilization != 0.91 {
		t.Errorf("per[five_hour].Utilization = %v, want 0.91", per[WindowFiveHour].Utilization)
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
	srv, _ := stubServer(t, http.StatusOK, `{"five_hour":{"utilization":50.0,"resets_at":"2026-04-28T12:00:00Z"}}`)
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
	srv, _ := stubServer(t, http.StatusOK, `{"five_hour":{"utilization":50.0,"resets_at":"2026-04-28T12:00:00Z"}}`)
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
