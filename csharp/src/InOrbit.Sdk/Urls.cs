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

        var host = url.IdnHost.Trim('[', ']');
        var loopback = host == "localhost" || (IPAddress.TryParse(host, out var ip) && IPAddress.IsLoopback(ip));
        if (!(url.Scheme == Uri.UriSchemeHttps || (url.Scheme == Uri.UriSchemeHttp && loopback)))
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
}
