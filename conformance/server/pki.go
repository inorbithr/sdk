package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"fmt"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"time"
)

// clientCN is the subject the client certificate written next to ca.pem carries; a case
// matches it with `client_cert: CN=conformance-client`.
const clientCN = "conformance-client"

// PKI is a throwaway certificate authority made when the server starts: a server leaf for
// the TLS and mTLS listeners, and a client certificate for mTLS. Nothing of it outlives
// the process except the files in Dir, which are removed on exit.
type PKI struct {
	Dir            string
	CAFile         string
	ClientCertFile string
	ClientKeyFile  string

	pool   *x509.CertPool
	server tls.Certificate
}

// newPKI generates the CA, the leaf and the client certificate (ECDSA P-256), and writes
// ca.pem, client.pem and client-key.pem into dir. Private keys of the CA and the server
// stay in memory; only the client key is written, readable by the owner alone.
func newPKI(dir string) (*PKI, error) {
	now := time.Now()
	caKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, err
	}
	caTmpl := &x509.Certificate{
		SerialNumber:          serial(),
		Subject:               pkix.Name{CommonName: "conformance replay CA"},
		NotBefore:             now.Add(-time.Hour),
		NotAfter:              now.Add(7 * 24 * time.Hour),
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
		BasicConstraintsValid: true,
		IsCA:                  true,
		MaxPathLenZero:        true,
	}
	caDER, err := x509.CreateCertificate(rand.Reader, caTmpl, caTmpl, &caKey.PublicKey, caKey)
	if err != nil {
		return nil, err
	}
	ca, err := x509.ParseCertificate(caDER)
	if err != nil {
		return nil, err
	}

	server, _, _, err := issue(ca, caKey, &x509.Certificate{
		Subject:     pkix.Name{CommonName: "localhost"},
		DNSNames:    []string{"localhost"},
		IPAddresses: []net.IP{net.IPv4(127, 0, 0, 1), net.IPv6loopback},
		ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
	}, now)
	if err != nil {
		return nil, err
	}
	_, clientDER, clientKey, err := issue(ca, caKey, &x509.Certificate{
		Subject:     pkix.Name{CommonName: clientCN},
		ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
	}, now)
	if err != nil {
		return nil, err
	}
	clientKeyDER, err := x509.MarshalPKCS8PrivateKey(clientKey)
	if err != nil {
		return nil, err
	}

	p := &PKI{
		Dir:            dir,
		CAFile:         filepath.Join(dir, "ca.pem"),
		ClientCertFile: filepath.Join(dir, "client.pem"),
		ClientKeyFile:  filepath.Join(dir, "client-key.pem"),
		pool:           x509.NewCertPool(),
		server:         server,
	}
	p.pool.AddCert(ca)
	for _, f := range []struct {
		path, kind string
		der        []byte
		mode       os.FileMode
	}{
		{p.CAFile, "CERTIFICATE", caDER, 0o644},
		{p.ClientCertFile, "CERTIFICATE", clientDER, 0o644},
		{p.ClientKeyFile, "PRIVATE KEY", clientKeyDER, 0o600},
	} {
		data := pem.EncodeToMemory(&pem.Block{Type: f.kind, Bytes: f.der})
		if err := os.WriteFile(f.path, data, f.mode); err != nil {
			return nil, fmt.Errorf("writing %s: %w", f.path, err)
		}
	}
	return p, nil
}

// issue signs a leaf for tmpl with the CA and returns it as a tls.Certificate, its DER
// and its key.
func issue(ca *x509.Certificate, caKey *ecdsa.PrivateKey, tmpl *x509.Certificate, now time.Time) (tls.Certificate, []byte, *ecdsa.PrivateKey, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return tls.Certificate{}, nil, nil, err
	}
	tmpl.SerialNumber = serial()
	tmpl.NotBefore = now.Add(-time.Hour)
	tmpl.NotAfter = now.Add(7 * 24 * time.Hour)
	tmpl.KeyUsage = x509.KeyUsageDigitalSignature
	tmpl.BasicConstraintsValid = true
	der, err := x509.CreateCertificate(rand.Reader, tmpl, ca, &key.PublicKey, caKey)
	if err != nil {
		return tls.Certificate{}, nil, nil, err
	}
	leaf, err := x509.ParseCertificate(der)
	if err != nil {
		return tls.Certificate{}, nil, nil, err
	}
	return tls.Certificate{Certificate: [][]byte{der, ca.Raw}, PrivateKey: key, Leaf: leaf}, der, key, nil
}

func serial() *big.Int {
	n, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 127))
	if err != nil {
		return big.NewInt(time.Now().UnixNano())
	}
	return n
}

// serverTLS is the TLS listener's configuration; requireClient makes it the mTLS one.
// HTTP/1.1 only, so an answer behaves the same over TLS as over plain HTTP (chunked
// bodies, resets, the socket upgrade).
func (p *PKI) serverTLS(requireClient bool) *tls.Config {
	cfg := &tls.Config{
		Certificates: []tls.Certificate{p.server},
		MinVersion:   tls.VersionTLS12,
		NextProtos:   []string{"http/1.1"},
	}
	if requireClient {
		cfg.ClientAuth = tls.RequireAndVerifyClientCert
		cfg.ClientCAs = p.pool
	}
	return cfg
}

// clientTLS trusts the CA and, with cert, presents the client certificate (the self-test).
func (p *PKI) clientTLS(cert bool) (*tls.Config, error) {
	cfg := &tls.Config{RootCAs: p.pool, MinVersion: tls.VersionTLS12}
	if cert {
		pair, err := tls.LoadX509KeyPair(p.ClientCertFile, p.ClientKeyFile)
		if err != nil {
			return nil, err
		}
		cfg.Certificates = []tls.Certificate{pair}
	}
	return cfg, nil
}
