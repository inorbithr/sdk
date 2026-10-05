package inorbit

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/pbkdf2"
	"crypto/sha1" //nolint:gosec // PKCS#5's default PRF, only to read keys written with it
	"crypto/sha256"
	"crypto/sha512"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/asn1"
	"encoding/base64"
	"encoding/pem"
	"errors"
	"fmt"
	"hash"
	"net"
	"net/http"
	"net/url"
	"os"
	"slices"
	"sync"
	"time"
)

// Transport settings (docs/config.md section 6): the proxy with the SDK's no_proxy
// grammar, the trust store with a CA bundle added, client certificates re-read when they
// change, pinning and the connect timeout.

// transportSettings are the settings that belong to the HTTP client.
var transportSettings = []string{
	"connect_timeout", "proxy", "no_proxy", "ca_bundle", "system_trust", "client_cert", "client_key",
	"client_key_password", "pinned_keys",
}

// httpClientFor is the client requests go through: the caller's (copied, redirects
// off), the default one for NewClient without transport settings as before, or one
// built from the settings.
func httpClientFor(cfg *config, res *resolution, explicit bool) (*http.Client, error) {
	switch {
	case cfg.http != nil:
		copied := *cfg.http
		copied.CheckRedirect = noRedirects
		return &copied, nil
	case cfg.transport != nil:
		return &http.Client{Transport: cfg.transport, CheckRedirect: noRedirects}, nil
	}
	if explicit && !slices.ContainsFunc(transportSettings, func(k string) bool { _, ok := cfg.code[k]; return ok }) {
		return &http.Client{CheckRedirect: noRedirects}, nil
	}
	t, err := newTransport(res)
	if err != nil {
		return nil, err
	}
	return &http.Client{Transport: t, CheckRedirect: noRedirects}, nil
}

func newTransport(res *resolution) (*http.Transport, error) {
	v := res.values
	connect, _ := v["connect_timeout"].(time.Duration)
	if connect <= 0 {
		connect = 10 * time.Second
	}
	entries, _ := v["no_proxy"].([]string)
	noProxy, _, _ := parseNoProxy(entries)
	choice := res.proxy
	tlsConfig, err := tlsConfigFor(v)
	if err != nil {
		return nil, err
	}
	dialer := &net.Dialer{Timeout: connect, KeepAlive: 30 * time.Second}
	return &http.Transport{
		Proxy: func(r *http.Request) (*url.URL, error) {
			p := proxyFor(r.URL, choice, noProxy)
			if p == "" {
				return nil, nil
			}
			return url.Parse(p)
		},
		DialContext:           dialer.DialContext,
		TLSClientConfig:       tlsConfig,
		TLSHandshakeTimeout:   connect,
		ForceAttemptHTTP2:     true,
		MaxIdleConns:          100,
		IdleConnTimeout:       90 * time.Second,
		ExpectContinueTimeout: time.Second,
	}, nil
}

func tlsConfigFor(v map[string]any) (*tls.Config, error) {
	cfg := &tls.Config{MinVersion: tls.VersionTLS12}
	if bundle, ok := v["ca_bundle"].(string); ok {
		pool := x509.NewCertPool()
		if trust, ok := v["system_trust"].(bool); !ok || trust {
			if sys, err := x509.SystemCertPool(); err == nil && sys != nil {
				pool = sys
			}
		}
		pemData, err := os.ReadFile(bundle) //nolint:gosec // the configured CA bundle
		if err != nil {
			return nil, &ConfigError{Message: "ca_bundle: cannot read " + bundle}
		}
		if !pool.AppendCertsFromPEM(pemData) {
			return nil, &ConfigError{Message: "ca_bundle: " + bundle + " holds no PEM certificate"}
		}
		cfg.RootCAs = pool
	}
	if cert, ok := v["client_cert"].(string); ok {
		key, _ := v["client_key"].(string)
		password, _ := v["client_key_password"].(string)
		r := &certReloader{certFile: cert, keyFile: key, password: password}
		if err := r.load(); err != nil {
			return nil, &ConfigError{Message: "client_cert: " + err.Error()}
		}
		cfg.GetClientCertificate = r.get
	}
	if pins, ok := v["pinned_keys"].([]string); ok && len(pins) > 0 {
		cfg.VerifyConnection = func(cs tls.ConnectionState) error {
			for _, chain := range cs.VerifiedChains {
				for _, c := range chain {
					sum := sha256.Sum256(c.RawSubjectPublicKeyInfo)
					if slices.Contains(pins, base64.StdEncoding.EncodeToString(sum[:])) {
						return nil
					}
				}
			}
			return fmt.Errorf("no certificate %s presented matches a pinned key", cs.ServerName)
		}
	}
	return cfg, nil
}

// certReloader holds the client certificate and reads it again when either file's
// modification time changes, checked at most once a minute, so cert-manager rotation
// needs no restart.
type certReloader struct {
	certFile, keyFile, password string

	mu      sync.Mutex
	cert    *tls.Certificate
	stamp   string
	checked time.Time
}

func (r *certReloader) stampNow() string {
	var s string
	for _, f := range []string{r.certFile, r.keyFile} {
		if info, err := os.Stat(f); err == nil {
			s += info.ModTime().String() + "|"
		}
	}
	return s
}

func (r *certReloader) load() error {
	certPEM, err := os.ReadFile(r.certFile)
	if err != nil {
		return fmt.Errorf("cannot read %s", r.certFile)
	}
	keyPEM, err := os.ReadFile(r.keyFile)
	if err != nil {
		return fmt.Errorf("cannot read %s", r.keyFile)
	}
	if block, _ := pem.Decode(keyPEM); block != nil && block.Type == "ENCRYPTED PRIVATE KEY" {
		if r.password == "" {
			return fmt.Errorf("%s is encrypted: set client_key_password", r.keyFile)
		}
		der, err := decryptPKCS8(block.Bytes, []byte(r.password))
		if err != nil {
			return fmt.Errorf("%s: %w", r.keyFile, err)
		}
		keyPEM = pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der})
	}
	cert, err := tls.X509KeyPair(certPEM, keyPEM)
	if err != nil {
		return fmt.Errorf("%s and %s are not a certificate and its key", r.certFile, r.keyFile)
	}
	r.mu.Lock()
	r.cert, r.stamp, r.checked = &cert, r.stampNow(), time.Now()
	r.mu.Unlock()
	return nil
}

func (r *certReloader) get(*tls.CertificateRequestInfo) (*tls.Certificate, error) {
	r.mu.Lock()
	due := time.Since(r.checked) >= time.Minute
	if due {
		r.checked = time.Now()
	}
	stamp := r.stamp
	r.mu.Unlock()
	if due && r.stampNow() != stamp {
		_ = r.load() // A half-written rotation keeps the certificate read last.
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.cert, nil
}

var (
	oidPBES2      = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 5, 13}
	oidPBKDF2     = asn1.ObjectIdentifier{1, 2, 840, 113549, 1, 5, 12}
	oidHMACSHA1   = asn1.ObjectIdentifier{1, 2, 840, 113549, 2, 7}
	oidHMACSHA256 = asn1.ObjectIdentifier{1, 2, 840, 113549, 2, 9}
	oidHMACSHA384 = asn1.ObjectIdentifier{1, 2, 840, 113549, 2, 10}
	oidHMACSHA512 = asn1.ObjectIdentifier{1, 2, 840, 113549, 2, 11}
	oidAES128CBC  = asn1.ObjectIdentifier{2, 16, 840, 1, 101, 3, 4, 1, 2}
	oidAES192CBC  = asn1.ObjectIdentifier{2, 16, 840, 1, 101, 3, 4, 1, 22}
	oidAES256CBC  = asn1.ObjectIdentifier{2, 16, 840, 1, 101, 3, 4, 1, 42}
)

type encryptedPrivateKeyInfo struct {
	Algorithm pkix.AlgorithmIdentifier
	Data      []byte
}

type pbes2Params struct {
	KeyDerivation pkix.AlgorithmIdentifier
	Encryption    pkix.AlgorithmIdentifier
}

type pbkdf2Params struct {
	Salt       []byte
	Iterations int
	KeyLength  int                      `asn1:"optional"`
	PRF        pkix.AlgorithmIdentifier `asn1:"optional"`
}

// decryptPKCS8 decrypts an encrypted PKCS#8 key (PBES2 with PBKDF2 and AES-CBC, what
// OpenSSL writes) into its DER PrivateKeyInfo.
func decryptPKCS8(der, password []byte) ([]byte, error) {
	var info encryptedPrivateKeyInfo
	if _, err := asn1.Unmarshal(der, &info); err != nil || !info.Algorithm.Algorithm.Equal(oidPBES2) {
		return nil, errors.New("the key is not PBES2-encrypted PKCS#8")
	}
	var params pbes2Params
	if _, err := asn1.Unmarshal(info.Algorithm.Parameters.FullBytes, &params); err != nil || !params.KeyDerivation.Algorithm.Equal(oidPBKDF2) {
		return nil, errors.New("the key's encryption is not PBKDF2")
	}
	var kdf pbkdf2Params
	if _, err := asn1.Unmarshal(params.KeyDerivation.Parameters.FullBytes, &kdf); err != nil {
		return nil, errors.New("the key's PBKDF2 parameters cannot be read")
	}
	var prf func() hash.Hash
	switch {
	case len(kdf.PRF.Algorithm) == 0, kdf.PRF.Algorithm.Equal(oidHMACSHA1):
		prf = sha1.New
	case kdf.PRF.Algorithm.Equal(oidHMACSHA256):
		prf = sha256.New
	case kdf.PRF.Algorithm.Equal(oidHMACSHA384):
		prf = sha512.New384
	case kdf.PRF.Algorithm.Equal(oidHMACSHA512):
		prf = sha512.New
	default:
		return nil, errors.New("the key's PBKDF2 PRF is not supported")
	}
	var size int
	switch {
	case params.Encryption.Algorithm.Equal(oidAES128CBC):
		size = 16
	case params.Encryption.Algorithm.Equal(oidAES192CBC):
		size = 24
	case params.Encryption.Algorithm.Equal(oidAES256CBC):
		size = 32
	default:
		return nil, errors.New("the key's cipher is not AES-CBC")
	}
	var iv []byte
	if _, err := asn1.Unmarshal(params.Encryption.Parameters.FullBytes, &iv); err != nil || len(iv) != aes.BlockSize {
		return nil, errors.New("the key's IV cannot be read")
	}
	key, err := pbkdf2.Key(prf, string(password), kdf.Salt, kdf.Iterations, size)
	if err != nil {
		return nil, errors.New("the key's PBKDF2 parameters are not usable")
	}
	block, err := aes.NewCipher(key)
	if err != nil || len(info.Data) == 0 || len(info.Data)%aes.BlockSize != 0 {
		return nil, errors.New("the key cannot be decrypted")
	}
	out := make([]byte, len(info.Data))
	cipher.NewCBCDecrypter(block, iv).CryptBlocks(out, info.Data)
	pad := int(out[len(out)-1])
	if pad == 0 || pad > aes.BlockSize || pad > len(out) {
		return nil, errors.New("the password is wrong")
	}
	for _, b := range out[len(out)-pad:] {
		if int(b) != pad {
			return nil, errors.New("the password is wrong")
		}
	}
	out = out[:len(out)-pad]
	if _, err := x509.ParsePKCS8PrivateKey(out); err != nil {
		return nil, errors.New("the password is wrong")
	}
	return out, nil
}
