using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Numerics;
using System.Text.RegularExpressions;

namespace InOrbit.Sdk;

/// <summary>
/// The <c>no_proxy</c> grammar every runtime shares (docs/config.md section 6.2), implemented here
/// rather than left to the platform, whose rules differ: <c>*</c>, a domain and its subdomains (a
/// leading dot changes nothing), <c>host:port</c>, an IP literal, or a CIDR range of IP literals.
/// </summary>
internal sealed partial class NoProxy
{
    private readonly IReadOnlyList<Entry> _entries;

    private NoProxy(IReadOnlyList<Entry> entries)
    {
        _entries = entries;
    }

    internal static NoProxy Empty { get; } = new([]);

    /// <summary>The entries; empty ones are skipped. A bad entry is a <see cref="FormatException"/>.</summary>
    internal static NoProxy Parse(IEnumerable<string> entries)
    {
        var list = new List<Entry>();
        foreach (var raw in entries)
        {
            if (raw.Trim().Length == 0)
            {
                continue;
            }

            list.Add(ParseEntry(raw) ?? throw new FormatException(raw));
        }

        return new NoProxy(list);
    }

    /// <summary>One entry, or <see langword="null"/> when it fits no form.</summary>
    internal static Entry? ParseEntry(string raw)
    {
        var e = raw.Trim().ToLowerInvariant();
        if (e == "*")
        {
            return new Entry(Kind.All, null, null, 0, null);
        }

        var slash = e.IndexOf('/', StringComparison.Ordinal);
        if (slash >= 0)
        {
            var bitsText = e[(slash + 1)..];
            var net = Ip(e[..slash]);
            if (net is not { } n || !BitsPattern().IsMatch(bitsText))
            {
                return null;
            }

            var bits = int.Parse(bitsText, CultureInfo.InvariantCulture);
            var width = n.V6 ? 128 : 32;
            if (bits > width)
            {
                return null;
            }

            return new Entry(Kind.Cidr, (n.V6, n.Value >> (width - bits)), null, bits, null);
        }

        if (Ip(e) is { } bare)
        {
            return new Entry(Kind.Ip, bare, null, 0, null);
        }

        var host = e;
        int? port = null;
        var colon = e.LastIndexOf(':');
        if (colon >= 0)
        {
            var h = e[..colon];
            if (!h.Contains(':', StringComparison.Ordinal) || h.EndsWith(']'))
            {
                var p = e[(colon + 1)..];
                if (!PortPattern().IsMatch(p) || int.Parse(p, CultureInfo.InvariantCulture) > 65535)
                {
                    return null;
                }

                host = h;
                port = int.Parse(p, CultureInfo.InvariantCulture);
            }
        }

        if (Ip(host) is { } ip)
        {
            return new Entry(Kind.Ip, ip, port, 0, null);
        }

        var name = host.TrimStart('.');
        if (name.Length == 0 || !name.Split('.').All(l => LabelPattern().IsMatch(l)))
        {
            return null;
        }

        return new Entry(Kind.Name, null, port, 0, name);
    }

    /// <summary>Whether <paramref name="url"/> goes direct.</summary>
    internal bool Bypasses(Uri url)
    {
        var host = url.Host.Trim('[', ']').ToLowerInvariant();
        var port = url.Port;
        var ip = Ip(host);
        foreach (var e in _entries)
        {
            var hit = e.Kind switch
            {
                Kind.All => true,
                Kind.Cidr => ip is { } a && e.Net is { } n && a.V6 == n.V6 && (a.Value >> ((a.V6 ? 128 : 32) - e.Bits)) == n.Value,
                Kind.Ip => ip is { } b && e.Net is { } m && b.V6 == m.V6 && b.Value == m.Value && (e.Port is null || e.Port == port),
                Kind.Name => (host == e.Name || host.EndsWith("." + e.Name, StringComparison.Ordinal)) && (e.Port is null || e.Port == port),
                _ => false,
            };
            if (hit)
            {
                return true;
            }
        }

        return false;
    }

    /// <summary>The proxy <paramref name="url"/> goes through, or <see langword="null"/> for a direct connection.</summary>
    internal static Uri? For(Uri url, ProxyChoice? proxy, NoProxy noProxy)
    {
        if (proxy is null || proxy.Url == "off")
        {
            return null;
        }

        if (!proxy.Explicit && IsLoopback(url.Host))
        {
            return null;
        }

        if (noProxy.Bypasses(url))
        {
            return null;
        }

        // The user-info is the proxy's credentials (SdkProxy.Credentials), never part of its address.
        var u = new Uri(proxy.Url);
        return u.UserInfo.Length == 0 ? u : new UriBuilder(u) { UserName = string.Empty, Password = string.Empty }.Uri;
    }

    internal static bool IsLoopback(string host)
    {
        var h = host.Trim('[', ']').ToLowerInvariant();
        if (h == "localhost")
        {
            return true;
        }

        return Ip(h) is { } ip && (ip.V6 ? ip.Value == BigInteger.One : ip.Value >> 24 == 127);
    }

    /// <summary>An IPv4 or IPv6 literal (brackets allowed), as a number.</summary>
    private static (bool V6, BigInteger Value)? Ip(string raw)
    {
        var s = raw.StartsWith('[') && raw.EndsWith(']') ? raw[1..^1] : raw;
        if (V4Pattern().IsMatch(s))
        {
            var parts = s.Split('.');
            if (parts.Any(p => (p.Length > 1 && p[0] == '0') || int.Parse(p, CultureInfo.InvariantCulture) > 255))
            {
                return null;
            }

            return (false, parts.Aggregate(BigInteger.Zero, (acc, p) => (acc << 8) | int.Parse(p, CultureInfo.InvariantCulture)));
        }

        if (s.Contains(':', StringComparison.Ordinal) && !s.Contains('%', StringComparison.Ordinal) && IPAddress.TryParse(s, out var a) && a.AddressFamily == AddressFamily.InterNetworkV6)
        {
            return (true, new BigInteger(a.GetAddressBytes(), isUnsigned: true, isBigEndian: true));
        }

        return null;
    }

    [GeneratedRegex("^[0-9]{1,3}$", RegexOptions.CultureInvariant)]
    private static partial Regex BitsPattern();

    [GeneratedRegex("^[0-9]{1,5}$", RegexOptions.CultureInvariant)]
    private static partial Regex PortPattern();

    [GeneratedRegex("^[a-z0-9_-]+$", RegexOptions.CultureInvariant)]
    private static partial Regex LabelPattern();

    [GeneratedRegex("^[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}$", RegexOptions.CultureInvariant)]
    private static partial Regex V4Pattern();

    internal enum Kind
    {
        All,
        Cidr,
        Ip,
        Name,
    }

    internal sealed record Entry(Kind Kind, (bool V6, BigInteger Value)? Net, int? Port, int Bits, string? Name);
}

/// <summary>The SDK's proxy choice as the <see cref="IWebProxy"/> a <c>SocketsHttpHandler</c> asks per request.</summary>
internal sealed class SdkProxy(ProxyChoice? choice, NoProxy noProxy) : IWebProxy
{
    private readonly ICredentials? _credentials = FromUrl(choice);

    public ICredentials? Credentials
    {
        get => _credentials;
        set => throw new NotSupportedException("the proxy's credentials come from its URL");
    }

    public Uri? GetProxy(Uri destination) => NoProxy.For(destination, choice, noProxy);

    public bool IsBypassed(Uri host) => GetProxy(host) is null;

    private static NetworkCredential? FromUrl(ProxyChoice? choice)
    {
        if (choice is null || choice.Url == "off" || !Uri.TryCreate(choice.Url, UriKind.Absolute, out var u) || u.UserInfo.Length == 0)
        {
            return null;
        }

        var i = u.UserInfo.IndexOf(':', StringComparison.Ordinal);
        var user = Uri.UnescapeDataString(i < 0 ? u.UserInfo : u.UserInfo[..i]);
        var password = i < 0 ? string.Empty : Uri.UnescapeDataString(u.UserInfo[(i + 1)..]);
        return new NetworkCredential(user, password);
    }
}
