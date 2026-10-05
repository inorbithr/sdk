package hr.inorbit.sdk;

import hr.inorbit.sdk.errors.ConfigException;
import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.net.Socket;
import java.net.URI;
import java.net.http.HttpClient;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.GeneralSecurityException;
import java.security.KeyFactory;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.security.PrivateKey;
import java.security.cert.Certificate;
import java.security.cert.CertificateException;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.security.spec.PKCS8EncodedKeySpec;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collection;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import javax.crypto.Cipher;
import javax.crypto.EncryptedPrivateKeyInfo;
import javax.crypto.SecretKeyFactory;
import javax.crypto.spec.PBEKeySpec;
import javax.net.ssl.KeyManager;
import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLEngine;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509ExtendedTrustManager;
import javax.net.ssl.X509TrustManager;

/**
 * The HTTP client {@code load} builds (docs/config.md section 6): the connect timeout, the SDK's
 * own proxy rule as a {@link java.net.ProxySelector}, and an {@link SSLContext} that adds {@code
 * ca_bundle} to the system's trust (or trusts it alone), presents a client certificate and checks
 * pinned keys. Redirects are never followed.
 */
final class Transport {

    private Transport() {}

    /** The settings the HTTP client is built from. */
    record Net(
            Duration connectTimeout,
            NoProxy proxy,
            String caBundle,
            boolean systemTrust,
            String clientCert,
            String clientKey,
            String clientKeyPassword,
            KeyStore clientKeyStore,
            char[] clientKeyStorePassword,
            List<String> pinnedKeys) {}

    static HttpClient build(URI base, Net net) {
        HttpClient.Builder b = HttpClient.newBuilder()
                .followRedirects(HttpClient.Redirect.NEVER)
                .connectTimeout(net.connectTimeout())
                .proxy(net.proxy())
                .version("http".equals(base.getScheme()) ? HttpClient.Version.HTTP_1_1 : HttpClient.Version.HTTP_2);
        SSLContext ssl = ssl(net);
        if (ssl != null) {
            b.sslContext(ssl);
        }
        return b.build();
    }

    /** The TLS context the settings ask for, or {@code null} for the runtime's default. */
    static SSLContext ssl(Net net) {
        boolean custom = net.caBundle() != null
                || !net.systemTrust()
                || net.clientCert() != null
                || net.clientKeyStore() != null
                || (net.pinnedKeys() != null && !net.pinnedKeys().isEmpty());
        if (!custom) {
            return null;
        }
        try {
            X509ExtendedTrustManager trust = trust(net.caBundle(), net.systemTrust());
            if (net.pinnedKeys() != null && !net.pinnedKeys().isEmpty()) {
                trust = new Pinned(trust, Set.copyOf(net.pinnedKeys()));
            }
            KeyManager[] keys = null;
            if (net.clientKeyStore() != null) {
                KeyManagerFactory kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
                kmf.init(net.clientKeyStore(), net.clientKeyStorePassword());
                keys = kmf.getKeyManagers();
            } else if (net.clientCert() != null) {
                keys = clientKeys(net.clientCert(), net.clientKey(), net.clientKeyPassword());
            }
            SSLContext ctx = SSLContext.getInstance("TLS");
            ctx.init(keys, new TrustManager[] {trust}, null);
            return ctx;
        } catch (GeneralSecurityException | IOException e) {
            throw new ConfigException("the TLS settings cannot be used: " + Engine.describe(e));
        }
    }

    private static X509ExtendedTrustManager trust(String caBundle, boolean systemTrust)
            throws GeneralSecurityException, IOException {
        KeyStore store = KeyStore.getInstance(KeyStore.getDefaultType());
        store.load(null, null);
        int n = 0;
        if (systemTrust) {
            TrustManagerFactory sys = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
            sys.init((KeyStore) null);
            for (TrustManager tm : sys.getTrustManagers()) {
                if (tm instanceof X509TrustManager x) {
                    for (X509Certificate c : x.getAcceptedIssuers()) {
                        store.setCertificateEntry("system-" + n++, c);
                    }
                }
            }
        }
        if (caBundle != null) {
            Collection<? extends Certificate> certs = certificates(caBundle);
            if (certs.isEmpty()) {
                throw new ConfigException("ca_bundle " + caBundle + " holds no PEM certificate");
            }
            for (Certificate c : certs) {
                store.setCertificateEntry("bundle-" + n++, c);
            }
        }
        TrustManagerFactory tmf = TrustManagerFactory.getInstance("PKIX");
        tmf.init(store);
        for (TrustManager tm : tmf.getTrustManagers()) {
            if (tm instanceof X509ExtendedTrustManager x) {
                return x;
            }
        }
        throw new GeneralSecurityException("no X.509 trust manager");
    }

    private static Collection<? extends Certificate> certificates(String path)
            throws IOException, CertificateException {
        byte[] pem = Files.readAllBytes(Path.of(path));
        return CertificateFactory.getInstance("X.509").generateCertificates(new ByteArrayInputStream(pem));
    }

    private static KeyManager[] clientKeys(String certPath, String keyPath, String password)
            throws GeneralSecurityException, IOException {
        List<Certificate> chain = new ArrayList<>(certificates(certPath));
        if (chain.isEmpty()) {
            throw new ConfigException("client_cert " + certPath + " holds no PEM certificate");
        }
        PrivateKey key = privateKey(Files.readString(Path.of(keyPath), StandardCharsets.US_ASCII), password, keyPath);
        char[] pw = new char[0];
        KeyStore ks = KeyStore.getInstance("PKCS12");
        ks.load(null, null);
        ks.setKeyEntry("client", key, pw, chain.toArray(new Certificate[0]));
        KeyManagerFactory kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
        kmf.init(ks, pw);
        return kmf.getKeyManagers();
    }

    private static byte[] pemBlock(String pem, String label) {
        String begin = "-----BEGIN " + label + "-----";
        int i = pem.indexOf(begin);
        if (i < 0) {
            return null;
        }
        int j = pem.indexOf("-----END " + label + "-----", i);
        if (j < 0) {
            return null;
        }
        return Base64.getMimeDecoder().decode(pem.substring(i + begin.length(), j));
    }

    private static PrivateKey privateKey(String pem, String password, String path)
            throws GeneralSecurityException, IOException {
        byte[] der = pemBlock(pem, "PRIVATE KEY");
        if (der == null) {
            byte[] enc = pemBlock(pem, "ENCRYPTED PRIVATE KEY");
            if (enc == null) {
                throw new ConfigException("client_key " + path
                        + " is not a PKCS#8 PEM key (BEGIN PRIVATE KEY or BEGIN ENCRYPTED PRIVATE KEY); "
                        + "convert it with `openssl pkcs8 -topk8`");
            }
            if (password == null) {
                throw new ConfigException("client_key " + path + " is encrypted: set client_key_password");
            }
            EncryptedPrivateKeyInfo info = new EncryptedPrivateKeyInfo(enc);
            SecretKeyFactory f = SecretKeyFactory.getInstance(info.getAlgName());
            Cipher cipher = Cipher.getInstance(info.getAlgName());
            cipher.init(
                    Cipher.DECRYPT_MODE,
                    f.generateSecret(new PBEKeySpec(password.toCharArray())),
                    info.getAlgParameters());
            der = info.getKeySpec(cipher).getEncoded();
        }
        for (String alg : List.of("EC", "RSA", "Ed25519", "EdDSA")) {
            try {
                return KeyFactory.getInstance(alg).generatePrivate(new PKCS8EncodedKeySpec(der));
            } catch (GeneralSecurityException e) {
                // Try the next algorithm.
            }
        }
        throw new ConfigException("client_key " + path + " is not an EC, RSA or Ed25519 key");
    }

    /** The base64 SHA-256 of a certificate's SubjectPublicKeyInfo (SR-06). */
    static String spki(Certificate c) throws GeneralSecurityException {
        return Base64.getEncoder()
                .encodeToString(MessageDigest.getInstance("SHA-256")
                        .digest(c.getPublicKey().getEncoded()));
    }

    /** Refuses a server whose chain matches no pinned key, after the usual checks. */
    static final class Pinned extends X509ExtendedTrustManager {
        private final X509ExtendedTrustManager delegate;
        private final Set<String> pins;

        Pinned(X509ExtendedTrustManager delegate, Set<String> pins) {
            this.delegate = delegate;
            this.pins = new HashSet<>(pins);
        }

        private void pinned(X509Certificate[] chain) throws CertificateException {
            for (X509Certificate c : chain) {
                try {
                    if (pins.contains(spki(c))) {
                        return;
                    }
                } catch (GeneralSecurityException e) {
                    throw new CertificateException("cannot hash the server's key", e);
                }
            }
            throw new CertificateException("no certificate in the chain matches pinned_keys");
        }

        @Override
        public void checkServerTrusted(X509Certificate[] chain, String authType, Socket socket)
                throws CertificateException {
            delegate.checkServerTrusted(chain, authType, socket);
            pinned(chain);
        }

        @Override
        public void checkServerTrusted(X509Certificate[] chain, String authType, SSLEngine engine)
                throws CertificateException {
            delegate.checkServerTrusted(chain, authType, engine);
            pinned(chain);
        }

        @Override
        public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException {
            delegate.checkServerTrusted(chain, authType);
            pinned(chain);
        }

        @Override
        public void checkClientTrusted(X509Certificate[] chain, String authType, Socket socket)
                throws CertificateException {
            delegate.checkClientTrusted(chain, authType, socket);
        }

        @Override
        public void checkClientTrusted(X509Certificate[] chain, String authType, SSLEngine engine)
                throws CertificateException {
            delegate.checkClientTrusted(chain, authType, engine);
        }

        @Override
        public void checkClientTrusted(X509Certificate[] chain, String authType) throws CertificateException {
            delegate.checkClientTrusted(chain, authType);
        }

        @Override
        public X509Certificate[] getAcceptedIssuers() {
            return delegate.getAcceptedIssuers();
        }
    }
}
