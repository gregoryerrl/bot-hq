// Package anthropic exposes a read-only client for the unauthenticated-by-
// design Claude Code OAuth usage endpoint. Slice 5 C1 (H-32) wires this
// into Emma's plan-usage producer so the trio can halt at 95% utilization
// before user-driven rebuild-for-autonomy.
package anthropic

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"runtime"
	"time"
)

// DefaultBaseAPIURL is the canonical Anthropic API host. Overridable via
// the BASE_API_URL env in operator deployments; tests pass the httptest
// server URL directly.
const DefaultBaseAPIURL = "https://api.anthropic.com"

// fetchTimeout is the per-call cap on /api/oauth/usage. 5s is well above
// observed p99 latency for the endpoint and short enough that a flake
// inside the 60s polling cadence cannot stack into a back-pressure window.
const fetchTimeout = 5 * time.Second

// keychainExpirySkew is the runway before the OAuth token's expiresAt at
// which Fetch returns ErrTokenExpired. Calling /api/oauth/usage with a
// just-expired token surfaces a 401; the skew exists so the caller can
// log+skip cleanly rather than treat the auth failure as an anomaly.
const keychainExpirySkew = 5 * time.Minute

// keychainService is the macOS Keychain "Service" attribute Claude Code
// uses to store its OAuth credential blob. Verified against cli.js 2.1.73
// published source.
const keychainService = "Claude Code-credentials"

// Window names for the /api/oauth/usage response. Returned by Fetch as
// `maxWindow` so the strip can label which limit is binding.
const (
	WindowFiveHour       = "five_hour"
	WindowSevenDay       = "seven_day"
	WindowSevenDaySonnet = "seven_day_sonnet"
	WindowSevenDayOpus   = "seven_day_opus"
)

// Window is a single utilization-window entry from /api/oauth/usage.
// Utilization is normalized 0-1 by the API; callers scale to 0-100 for
// display.
type Window struct {
	Utilization float64   `json:"utilization"`
	ResetsAt    time.Time `json:"resets_at"`
}

// Credentials is the subset of the keychain-stored OAuth blob this package
// consumes — access token plus its absolute expiry. The published cli.js
// keychain layout exposes additional fields (refresh token, scopes); v1
// does not implement refresh, so the surface stays minimal.
type Credentials struct {
	AccessToken string
	ExpiresAt   time.Time
}

// CredentialSource resolves the Claude Code OAuth credential blob. The
// production implementation shells out to /usr/bin/security on macOS;
// tests inject a fake to avoid touching the host keychain.
type CredentialSource interface {
	Get(ctx context.Context) (Credentials, error)
}

// ErrTokenExpired is returned by Fetch when the keychain credential is at
// or within keychainExpirySkew of its expiresAt. Caller policy: skip the
// poll, log once, leave the prior published HubSnapshot intact.
var ErrTokenExpired = errors.New("anthropic oauth: token expired or near-expiry")

// ErrUnsupportedPlatform is returned by the default keychain source on
// non-darwin hosts. Caller policy: log once, publish PlanUsagePct=-1, do
// not retry.
var ErrUnsupportedPlatform = errors.New("anthropic oauth: keychain access only implemented for darwin")

// rawCredentialBlob mirrors the JSON shape `security` prints when fed the
// Claude Code-credentials service. Only the OAuth subtree is consumed.
type rawCredentialBlob struct {
	ClaudeAIOAuth struct {
		AccessToken string `json:"accessToken"`
		// ExpiresAt is encoded as a unix-ms integer in cli.js 2.1.73.
		ExpiresAt int64 `json:"expiresAt"`
	} `json:"claudeAiOauth"`
}

// rawUsageResponse mirrors the /api/oauth/usage JSON shape. Each window is
// optional — Opus-tier accounts include seven_day_opus; lower tiers may
// omit it. Decode-tolerant: missing windows are absent from perWindow and
// excluded from the maxUtil computation.
type rawUsageResponse struct {
	FiveHour       *Window `json:"five_hour,omitempty"`
	SevenDay       *Window `json:"seven_day,omitempty"`
	SevenDaySonnet *Window `json:"seven_day_sonnet,omitempty"`
	SevenDayOpus   *Window `json:"seven_day_opus,omitempty"`
}

// UsageClient fetches /api/oauth/usage with bearer auth pulled from a
// CredentialSource. Concurrent-safe — http.Client and the credential
// source are both safe under concurrent Get/Do calls.
type UsageClient struct {
	baseURL string
	creds   CredentialSource
	httpc   *http.Client
	nowFn   func() time.Time
}

// NewUsageClient constructs a client. baseURL defaults to
// DefaultBaseAPIURL when empty so the production wiring is a one-arg call.
// nowFn defaults to time.Now; tests override to stage near-expiry
// scenarios deterministically.
func NewUsageClient(baseURL string, creds CredentialSource) *UsageClient {
	if baseURL == "" {
		baseURL = DefaultBaseAPIURL
	}
	return &UsageClient{
		baseURL: baseURL,
		creds:   creds,
		httpc:   &http.Client{Timeout: fetchTimeout},
		nowFn:   time.Now,
	}
}

// SetNow overrides the clock used for token-expiry comparisons. Test-only
// hook; production callers leave the default.
func (c *UsageClient) SetNow(fn func() time.Time) {
	if fn != nil {
		c.nowFn = fn
	}
}

// Fetch returns the maximum utilization across all returned windows, the
// name of the binding window, and the per-window detail map. Sentinel
// errors:
//
//   - ErrTokenExpired: keychain credential is past keychainExpirySkew of
//     its absolute expiresAt; do not call the API.
//   - ErrUnsupportedPlatform: surfaces from the default keychain source on
//     non-darwin hosts.
//
// Other errors (network, 5xx, malformed JSON, auth-fail) are wrapped and
// returned as-is — caller applies backoff per its policy.
func (c *UsageClient) Fetch(ctx context.Context) (float64, string, map[string]Window, error) {
	creds, err := c.creds.Get(ctx)
	if err != nil {
		return 0, "", nil, err
	}
	if !creds.ExpiresAt.IsZero() && c.nowFn().Add(keychainExpirySkew).After(creds.ExpiresAt) {
		return 0, "", nil, ErrTokenExpired
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.baseURL+"/api/oauth/usage", nil)
	if err != nil {
		return 0, "", nil, fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Authorization", "Bearer "+creds.AccessToken)
	req.Header.Set("Accept", "application/json")

	resp, err := c.httpc.Do(req)
	if err != nil {
		return 0, "", nil, fmt.Errorf("fetch usage: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden {
		return 0, "", nil, fmt.Errorf("auth failed: status %d", resp.StatusCode)
	}
	if resp.StatusCode >= 500 {
		return 0, "", nil, fmt.Errorf("server error: status %d", resp.StatusCode)
	}
	if resp.StatusCode != http.StatusOK {
		return 0, "", nil, fmt.Errorf("unexpected status %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return 0, "", nil, fmt.Errorf("read body: %w", err)
	}
	var raw rawUsageResponse
	if err := json.Unmarshal(body, &raw); err != nil {
		return 0, "", nil, fmt.Errorf("decode body: %w", err)
	}

	perWindow := map[string]Window{}
	if raw.FiveHour != nil {
		perWindow[WindowFiveHour] = *raw.FiveHour
	}
	if raw.SevenDay != nil {
		perWindow[WindowSevenDay] = *raw.SevenDay
	}
	if raw.SevenDaySonnet != nil {
		perWindow[WindowSevenDaySonnet] = *raw.SevenDaySonnet
	}
	if raw.SevenDayOpus != nil {
		perWindow[WindowSevenDayOpus] = *raw.SevenDayOpus
	}

	maxUtil := -1.0
	maxWindow := ""
	// Iterate in fixed order so ties (rare but possible at e.g. brand-new
	// account with all-zero utilization) resolve deterministically. The
	// tie-break ordering — five_hour → seven_day → seven_day_sonnet →
	// seven_day_opus — sorts the most-frequently-binding window first so
	// the strip's tag stays readable across normal operation.
	for _, name := range []string{WindowFiveHour, WindowSevenDay, WindowSevenDaySonnet, WindowSevenDayOpus} {
		w, ok := perWindow[name]
		if !ok {
			continue
		}
		if w.Utilization > maxUtil {
			maxUtil = w.Utilization
			maxWindow = name
		}
	}
	if maxWindow == "" {
		return 0, "", nil, errors.New("usage response had no recognized windows")
	}
	return maxUtil, maxWindow, perWindow, nil
}

// KeychainCredentialSource is the production CredentialSource: shells out
// to /usr/bin/security to read the Claude Code-credentials blob. macOS
// only — non-darwin hosts surface ErrUnsupportedPlatform on Get.
type KeychainCredentialSource struct {
	// SecurityPath overrides the binary path for tests that want to stub
	// the shell-out without faking the whole CredentialSource interface.
	// Empty defaults to "/usr/bin/security".
	SecurityPath string
	// runner abstracts exec.CommandContext for hermetic tests. Production
	// path leaves it nil and uses the default runner.
	runner func(ctx context.Context, name string, args ...string) ([]byte, error)
}

// Get reads and parses the keychain credential blob. Always returns
// ErrUnsupportedPlatform on non-darwin hosts so the caller short-circuits
// to PlanUsagePct=-1 without spawning any subprocess.
func (k *KeychainCredentialSource) Get(ctx context.Context) (Credentials, error) {
	if runtime.GOOS != "darwin" {
		return Credentials{}, ErrUnsupportedPlatform
	}
	bin := k.SecurityPath
	if bin == "" {
		bin = "/usr/bin/security"
	}
	run := k.runner
	if run == nil {
		run = func(ctx context.Context, name string, args ...string) ([]byte, error) {
			return exec.CommandContext(ctx, name, args...).Output()
		}
	}
	out, err := run(ctx, bin, "find-generic-password", "-s", keychainService, "-w")
	if err != nil {
		return Credentials{}, fmt.Errorf("keychain read: %w", err)
	}
	return parseKeychainBlob(out)
}

// parseKeychainBlob extracts AccessToken + ExpiresAt from the JSON blob
// `security ... -w` prints to stdout. Trims trailing whitespace (the CLI
// appends a newline) before decoding. Exposed for tests.
func parseKeychainBlob(raw []byte) (Credentials, error) {
	for len(raw) > 0 && (raw[len(raw)-1] == '\n' || raw[len(raw)-1] == '\r' || raw[len(raw)-1] == ' ') {
		raw = raw[:len(raw)-1]
	}
	var blob rawCredentialBlob
	if err := json.Unmarshal(raw, &blob); err != nil {
		return Credentials{}, fmt.Errorf("decode keychain blob: %w", err)
	}
	if blob.ClaudeAIOAuth.AccessToken == "" {
		return Credentials{}, errors.New("keychain blob missing claudeAiOauth.accessToken")
	}
	c := Credentials{AccessToken: blob.ClaudeAIOAuth.AccessToken}
	if blob.ClaudeAIOAuth.ExpiresAt > 0 {
		c.ExpiresAt = time.UnixMilli(blob.ClaudeAIOAuth.ExpiresAt)
	}
	return c, nil
}
