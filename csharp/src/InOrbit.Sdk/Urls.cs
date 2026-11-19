using System;
using System.Net;

namespace InOrbit.Sdk;

/// <summary>The rule every URL the SDK calls obeys: https, plain http only to this machine.</summary>
internal static class Urls
{
    internal static Uri Check(string what, Uri url, bool originOnly)
    {
        if (!url.IsAbsoluteUri)
        {
            throw new ConfigException($"{what} is not usable: it is not an absolute URL");
        }

        if (!(url.Scheme == Uri.UriSchemeHttps || (url.Scheme == Uri.UriSchemeHttp && IsLoopback(url.IdnHost))))
        {
            throw new ConfigException($"{what} is not usable: it must use https (plain http only to this machine)");
        }

        if (url.UserInfo.Length > 0 || url.Fragment.Length > 0)
        {
            throw new ConfigException($"{what} is not usable: it must not carry credentials or a fragment");
        }

        if (originOnly && (url.AbsolutePath != "/" || url.Query.Length > 0))
        {
            throw new ConfigException($"{what} is not usable: it is an origin only, such as https://api.inorbit.hr");
        }

        return url;
    }

    /// <summary>Whether <paramref name="host"/> (a URL's host, brackets or not) is this machine: <c>localhost</c> or a loopback address, never a name that only starts like one.</summary>
    internal static bool IsLoopback(string host)
    {
        var h = host.Trim('[', ']').ToLowerInvariant();
        return h == "localhost" || (IPAddress.TryParse(h, out var ip) && IPAddress.IsLoopback(ip));
    }
}
