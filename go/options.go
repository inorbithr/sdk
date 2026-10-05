package inorbit

import (
	"log/slog"
	"net/http"
	"time"
)

// config is what the options set: catalogue settings set in code, by name, and the
// options that hold objects.
type config struct {
	code           map[string]any
	provider       TokenProvider
	http           *http.Client
	transport      http.RoundTripper
	hooks          []Hook
	pipeline       []func(*Pipeline)
	logger         *slog.Logger
	redact         Redact
	telemetry      Telemetry
	load           LoadOptions
	profileType    string
	budgetCapacity int
}

func newConfig(opts []Option) *config {
	c := &config{code: map[string]any{}}
	for _, o := range opts {
		if o != nil {
			o(c)
		}
	}
	return c
}

func (c *config) input(explicit bool) resolveInput {
	return resolveInput{
		code: c.code, httpClient: c.http != nil || c.transport != nil, tokenProvider: c.provider != nil,
		profileType: c.profileType, explicit: explicit, load: c.load,
	}
}

func set(name string, v any) Option { return func(c *config) { c.code[name] = v } }

// Option configures a Client. The options that set a setting of docs/config.md section 3
// are its source "code", which wins over the environment and the config file in Load.
type Option func(*config)

// WithToken authenticates with an API token (from the console or iohr token create).
func WithToken(token string) Option { return set("token", token) }

// WithKey authenticates with an API key, exchanged for short-lived tokens; it needs
// WithScopes.
func WithKey(id, secret string) Option {
	return func(c *config) { c.code["key_id"], c.code["key_secret"] = id, secret }
}

// WithKeyFile authenticates with an API key whose secret is in a file, read before every
// token exchange so a rotated secret is used at the next one; it needs WithScopes.
func WithKeyFile(id, secretFile string) Option {
	return func(c *config) { c.code["key_id"], c.code["key_secret_file"] = id, secretFile }
}

// WithTokenFile authenticates with a bearer token read from a file, read again when it
// changes and after the API refused it.
func WithTokenFile(path string) Option { return set("token_file", path) }

// WithScopes sets the scopes to ask for with a key, a subset of the key's; there is no
// default.
func WithScopes(scopes ...string) Option {
	return set("scopes", append([]string{}, scopes...))
}

// WithTokenProvider authenticates with your own token source.
func WithTokenProvider(p TokenProvider) Option { return func(c *config) { c.provider = p } }

// WithBaseURL sets the API's origin (default https://api.inorbit.hr; plain HTTP only to
// this machine).
func WithBaseURL(u string) Option { return set("base_url", u) }

// WithTokenURL sets the token endpoint a key is exchanged at.
func WithTokenURL(u string) Option { return set("token_url", u) }

// WithTimeout sets how long each attempt may take, its whole answer included (default
// 30 s).
func WithTimeout(d time.Duration) Option { return set("timeout", d) }

// WithMaxRetries sets the retries after the first attempt (default 2; 0 disables).
func WithMaxRetries(n int) Option { return set("max_retries", max(n, 0)) }

// WithUserAgentSuffix appends s to the user agent: RFC 9110 product tokens, at most 128
// characters.
func WithUserAgentSuffix(s string) Option { return set("user_agent_suffix", s) }

// WithHook adds an observer of every attempt.
func WithHook(h Hook) Option { return func(c *config) { c.hooks = append(c.hooks, h) } }

// WithHTTPClient sets the HTTP client: your transport, proxy and TLS settings. It is
// copied, and its Transport is the innermost step of the pipeline. The SDK does not
// follow redirects whatever the client's policy. Proxy, trust, mTLS, pinning and
// connect timeout settings belong to it then: set in code next to it they are a
// ConfigError, from the environment or the file they are ignored.
func WithHTTPClient(h *http.Client) Option { return func(c *config) { c.http = h } }

// WithTransport sets the innermost http.RoundTripper, as WithHTTPClient does with a
// client.
func WithTransport(rt http.RoundTripper) Option { return func(c *config) { c.transport = rt } }

// WithStreams sets how streams open: StreamsSSE (the default), a server-sent events
// request each, or StreamsSocket, every stream of the client over one /v1/ws socket.
func WithStreams(s Streams) Option { return set("streams", string(s)) }

// WithStreamIdleTimeout sets how long a stream may be silent, not even a keep-alive,
// before it fails with a TimeoutError or, on the socket, reconnects (default 45 s).
func WithStreamIdleTimeout(d time.Duration) Option { return set("stream_idle_timeout", d) }

// WithProfile chooses the config file's profile for the public client (Load), over
// INORBIT_PROFILE and the file's default.
func WithProfile(name string) Option { return set("profile", name) }

// WithProfileType resolves as the typed profile name, as a generated surface's Load
// does: the environment prefix INORBIT_<NAME>_ and the table [profiles.<name>], which
// neither WithProfile nor INORBIT_PROFILE can point elsewhere.
func WithProfileType(name string) Option { return func(c *config) { c.profileType = name } }

// WithConfigFile sets the config file Load reads; "off" reads none.
func WithConfigFile(path string) Option { return set("config_file", path) }

// WithLoadOptions sets what Load reads instead of the process (the environment, the OS,
// the home and working directories): for tests.
func WithLoadOptions(o LoadOptions) Option { return func(c *config) { c.load = o } }

// WithCredentialSources narrows the credential chain to these sources, in its fixed
// order: env, workload, file, cli. Code is always allowed.
func WithCredentialSources(sources ...string) Option {
	return set("credential_sources", append([]string{}, sources...))
}

// WithCLIPath sets the iohr command line the cli credential source runs.
func WithCLIPath(path string) Option { return set("cli_path", path) }

// WithConnectTimeout sets how long DNS, TCP and the TLS handshake may take per new
// connection (default 10 s).
func WithConnectTimeout(d time.Duration) Option { return set("connect_timeout", d) }

// WithTotalTimeout sets how long one call may take, every attempt and wait included
// (default 120 s). A shorter deadline on the call's context wins.
func WithTotalTimeout(d time.Duration) Option { return set("total_timeout", d) }

// WithRetryBaseDelay sets the exponential backoff's base (default 500 ms).
func WithRetryBaseDelay(d time.Duration) Option { return set("retry_base_delay", d) }

// WithRetryMaxDelay sets the backoff's cap (default 8 s).
func WithRetryMaxDelay(d time.Duration) Option { return set("retry_max_delay", d) }

// WithRetryAfterMax sets the longest Retry-After the client waits (default 60 s); a
// longer one ends the call with the error.
func WithRetryAfterMax(d time.Duration) Option { return set("retry_after_max", d) }

// WithRetryBudget switches the per-client retry budget on or off (default on).
func WithRetryBudget(on bool) Option { return set("retry_budget", on) }

// WithRetryBudgetCapacity sets the retry budget's capacity (default 500), for tests.
func WithRetryBudgetCapacity(n int) Option { return func(c *config) { c.budgetCapacity = n } }

// WithProxy sets the proxy every request goes through, http:// or https://, or "off" to
// use none, the standard variables included. User-info in the URL is sent as
// Proxy-Authorization on CONNECT.
func WithProxy(u string) Option { return set("proxy", u) }

// WithNoProxy sets the hosts reached directly: names (and their subdomains), host:port,
// IP addresses, CIDR ranges, or * for every host.
func WithNoProxy(entries ...string) Option { return set("no_proxy", append([]string{}, entries...)) }

// WithCABundle adds the PEM certificates in path to the trust store.
func WithCABundle(path string) Option { return set("ca_bundle", path) }

// WithSystemTrust set to false trusts the CA bundle only, for a private gateway.
func WithSystemTrust(on bool) Option { return set("system_trust", on) }

// WithClientCert presents the PEM certificate chain in cert and the key in key for mTLS;
// both are read again when they change, checked at most once a minute.
func WithClientCert(cert, key string) Option {
	return func(c *config) { c.code["client_cert"], c.code["client_key"] = cert, key }
}

// WithClientKeyPassword decrypts an encrypted PKCS#8 client key.
func WithClientKeyPassword(password string) Option { return set("client_key_password", password) }

// WithPinnedKeys pins the API's public keys: base64 SHA-256 of each SPKI, at least two
// (the current one and a backup).
func WithPinnedKeys(pins ...string) Option { return set("pinned_keys", append([]string{}, pins...)) }

// WithLog sets how much is logged (default LogOff); records go to WithLogger's logger,
// or slog.Default.
func WithLog(level LogLevel) Option { return set("log", string(level)) }

// WithLogHeaders logs allowlisted header values at debug.
func WithLogHeaders(on bool) Option { return set("log_headers", on) }

// WithLogAllowHeaders adds header names to the logging allowlist; the never-logged
// headers stay out.
func WithLogAllowHeaders(names ...string) Option {
	return set("log_allow_headers", append([]string{}, names...))
}

// WithLogger sets where records go.
func WithLogger(l *slog.Logger) Option { return func(c *config) { c.logger = l } }

// WithRedact sets the last step before the logger: it sees every record.
func WithRedact(f Redact) Option { return func(c *config) { c.redact = f } }

// WithTracing switches spans on or off; on by default once a tracer is installed.
func WithTracing(on bool) Option { return set("tracing", on) }

// WithMetrics switches metrics on or off; by default as tracing.
func WithMetrics(on bool) Option { return set("metrics", on) }

// WithTelemetry installs a tracer and a meter, as inorbitotel.Pipeline does.
func WithTelemetry(t Telemetry) Option { return func(c *config) { c.telemetry = t } }

// WithRateLimit sets what the rate_limit middleware does (default RateLimitObserve).
func WithRateLimit(mode RateLimitMode) Option { return set("rate_limit", string(mode)) }

// WithPipeline edits the client's pipeline by name; several apply in order.
func WithPipeline(edit func(*Pipeline)) Option {
	return func(c *config) { c.pipeline = append(c.pipeline, edit) }
}
