using System;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Security;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

namespace InOrbit.Sdk;

/// <summary>The SDK's own HTTP handler: the connect timeout, the proxy, trust, mTLS and pinning (docs/config.md section 6).</summary>
internal static class Network
{
    internal static SocketsHttpHandler Handler(Resolution res, X509Certificate2Collection? clientCertificates)
    {
        var handler = new SocketsHttpHandler
        {
            AllowAutoRedirect = false,
            ConnectTimeout = res.Duration("connect_timeout") is var c && c > TimeSpan.Zero ? c : TimeSpan.FromSeconds(10),
            AutomaticDecompression = DecompressionMethods.GZip,
        };

        // The SDK chooses the proxy itself: the platform's default would read HTTP_PROXY and ALL_PROXY.
        var proxy = res.Proxy;
        if (proxy is null || proxy.Url == "off")
        {
            handler.UseProxy = false;
        }
        else
        {
            handler.UseProxy = true;
            handler.Proxy = new SdkProxy(proxy, NoProxy.Parse(res.Get<string[]>("no_proxy") ?? []));
        }

        var caBundle = res.Get<string>("ca_bundle");
        var systemTrust = !res.Values.TryGetValue("system_trust", out var st) || (bool)st;
        var pins = res.Get<string[]>("pinned_keys");
        if (caBundle is not null || !systemTrust || pins is not null)
        {
            var extra = new X509Certificate2Collection();
            if (caBundle is not null)
            {
                try
                {
                    extra.ImportFromPemFile(caBundle);
                }
                catch (Exception e) when (e is CryptographicException or System.IO.IOException or UnauthorizedAccessException)
                {
                    throw ConfigException.Of([new("ca_bundle", string.Empty, $"cannot read the certificates in {caBundle}: {e.Message}")]);
                }

                if (extra.Count == 0)
                {
                    throw ConfigException.Of([new("ca_bundle", string.Empty, $"{caBundle} holds no PEM certificate")]);
                }
            }

            handler.SslOptions.RemoteCertificateValidationCallback = (_, cert, chain, errors) => Verify(cert, chain, errors, extra, systemTrust, pins);
        }

        var certificates = clientCertificates ?? ClientCertificate(res);
        if (certificates is not null && certificates.Count > 0)
        {
            handler.SslOptions.ClientCertificates = certificates;
            handler.SslOptions.LocalCertificateSelectionCallback = (_, _, _, _, _) => certificates[0];
        }

        return handler;
    }

    /// <summary>The system's verdict, or the chain built to <paramref name="extra"/>; then the pins, against any certificate of the chain.</summary>
    private static bool Verify(X509Certificate? cert, X509Chain? chain, SslPolicyErrors errors, X509Certificate2Collection extra, bool systemTrust, string[]? pins)
    {
        if (cert is null || (errors & (SslPolicyErrors.RemoteCertificateNameMismatch | SslPolicyErrors.RemoteCertificateNotAvailable)) != 0)
        {
            return false;
        }

        using var leaf = new X509Certificate2(cert);
        X509Chain? built = null;
        try
        {
            var trusted = systemTrust && errors == SslPolicyErrors.None;
            if (!trusted && extra.Count > 0)
            {
                built = new X509Chain();
                built.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
                built.ChainPolicy.CustomTrustStore.AddRange(extra);
                built.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
                if (chain is not null)
                {
                    foreach (var element in chain.ChainElements)
                    {
                        built.ChainPolicy.ExtraStore.Add(element.Certificate);
                    }
                }

                trusted = built.Build(leaf);
            }

            if (!trusted)
            {
                return false;
            }

            if (pins is null)
            {
                return true;
            }

            var used = built ?? chain;
            var certs = used is null ? [leaf] : used.ChainElements.Select(e => e.Certificate).Prepend(leaf);
            return certs.Any(c => Array.IndexOf(pins, Convert.ToBase64String(SHA256.HashData(c.PublicKey.ExportSubjectPublicKeyInfo()))) >= 0);
        }
        finally
        {
            built?.Dispose();
        }
    }

    /// <summary>The PEM client certificate and key, read at load (an encrypted PKCS#8 key with its password).</summary>
    private static X509Certificate2Collection? ClientCertificate(Resolution res)
    {
        var certPath = res.Get<string>("client_cert");
        var keyPath = res.Get<string>("client_key");
        if (certPath is null || keyPath is null)
        {
            return null;
        }

        try
        {
            var password = res.Get<string>("client_key_password");
            var cert = password is null
                ? X509Certificate2.CreateFromPemFile(certPath, keyPath)
                : X509Certificate2.CreateFromEncryptedPemFile(certPath, password, keyPath);
            if (OperatingSystem.IsWindows())
            {
                // SChannel cannot use an ephemeral key: give it a PKCS#12 copy.
                using var ephemeral = cert;
#pragma warning disable SYSLIB0057 // The loader API arrives with .NET 9; this targets .NET 8.
                return [new X509Certificate2(ephemeral.Export(X509ContentType.Pkcs12))];
#pragma warning restore SYSLIB0057
            }

            return [cert];
        }
        catch (Exception e) when (e is CryptographicException or System.IO.IOException or UnauthorizedAccessException or ArgumentException)
        {
            throw ConfigException.Of([new("client_cert", string.Empty, $"cannot read the client certificate {certPath} with its key {keyPath}: {e.Message}")]);
        }
    }
}
