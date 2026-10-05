# Changelog

## 0.2.2 (2026-10-05)


### Features

* **conformance:** replay vectors prints the vectors as JSON for drivers without a YAML reader ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** calls have a total timeout of 120 s by default (WithTotalTimeout), every attempt and wait included ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** inorbit.Load with one precedence (code, environment, the iohr config file, defaults), Describe and ConfigError problems ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** OpenTelemetry spans and metrics in the optional module github.com/inorbithr/sdk/go/otel ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own http.Transport, or a caller's client or RoundTripper ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the named middleware pipeline over http.RoundTripper with its built-ins, log/slog logging, rate limits and the retry budget ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
