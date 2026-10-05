package hr.inorbit.sdk;

import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Proxy;
import java.net.ProxySelector;
import java.net.SocketAddress;
import java.net.URI;
import java.net.UnknownHostException;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * Which proxy, if any, a URL goes through (docs/config.md section 6.2): the SDK's own {@code
 * no_proxy} grammar, the same in every runtime, and a {@link ProxySelector} that applies it.
 * Names are never resolved to check a range.
 */
final class NoProxy extends ProxySelector {

    private final URI proxy;
    private final boolean explicit;
    private final List<String> entries;

    /**
     * A rule.
     *
     * @param proxy the proxy URL, user-info included; {@code null} for none
     * @param explicit set in code, {@code INORBIT_PROXY} or the file: it applies to loopback too
     * @param entries the {@code no_proxy} entries, already checked
     */
    NoProxy(String proxy, boolean explicit, List<String> entries) {
        this.proxy = proxy == null ? null : URI.create(proxy);
        this.explicit = explicit;
        this.entries = List.copyOf(entries);
    }

    /** The rule the resolved {@code proxy} and {@code no_proxy} settings give. */
    @SuppressWarnings("unchecked")
    static NoProxy of(Map<String, Object> values, String proxySource) {
        Object proxy = values.get("proxy");
        List<String> entries = values.get("no_proxy") instanceof List<?> l ? (List<String>) l : List.of();
        if (proxy == null || proxy.equals("off")) {
            return new NoProxy(null, false, entries);
        }
        boolean explicit = !(proxySource.equals("env https_proxy") || proxySource.equals("env HTTPS_PROXY"));
        return new NoProxy(proxy.toString(), explicit, entries);
    }

    /** The proxy URL, user-info included, or {@code null}. */
    URI proxy() {
        return proxy;
    }

    /** {@code Proxy-Authorization} for the proxy's user-info, or {@code null}. */
    String authorization() {
        if (proxy == null || proxy.getRawUserInfo() == null) {
            return null;
        }
        String info = java.net.URLDecoder.decode(
                proxy.getRawUserInfo().replace("+", "%2B"), java.nio.charset.StandardCharsets.UTF_8);
        return "Basic "
                + java.util.Base64.getEncoder().encodeToString(info.getBytes(java.nio.charset.StandardCharsets.UTF_8));
    }

    /**
     * The proxy URL for {@code url}, or {@code null} for a direct connection.
     *
     * @param url the URL to reach
     * @return the proxy, or {@code null}
     */
    URI proxyFor(URI url) {
        if (proxy == null) {
            return null;
        }
        String host = Config.strip(url.getHost() == null ? "" : url.getHost()).toLowerCase(Locale.ROOT);
        if (!explicit && Config.isLoopback(host)) {
            return null;
        }
        int port = url.getPort() >= 0 ? url.getPort() : "https".equals(url.getScheme()) ? 443 : 80;
        byte[] address = literal(host);
        for (String raw : entries) {
            String e = raw.strip().toLowerCase(Locale.ROOT);
            if (e.isEmpty()) {
                continue;
            }
            if (e.equals("*")) {
                return null;
            }
            if (e.contains("/")) {
                if (address != null && inCidr(address, e)) {
                    return null;
                }
                continue;
            }
            byte[] bare = literal(Config.strip(e));
            if (bare != null) {
                if (address != null && java.util.Arrays.equals(bare, address)) {
                    return null;
                }
                continue;
            }
            String h = e;
            Integer p = null;
            int colon = e.lastIndexOf(':');
            if (colon >= 0 && (e.indexOf(':') == colon || e.substring(0, colon).endsWith("]"))) {
                h = e.substring(0, colon);
                p = Integer.valueOf(e.substring(colon + 1));
            }
            if (p != null && p != port) {
                continue;
            }
            h = Config.strip(h);
            while (h.startsWith(".")) {
                h = h.substring(1);
            }
            byte[] hostAddress = literal(h);
            if (hostAddress != null) {
                if (address != null && java.util.Arrays.equals(hostAddress, address)) {
                    return null;
                }
                continue;
            }
            if (!h.isEmpty() && (host.equals(h) || host.endsWith("." + h))) {
                return null;
            }
        }
        return proxy;
    }

    @Override
    public List<Proxy> select(URI uri) {
        URI p = proxyFor(uri);
        if (p == null) {
            return List.of(Proxy.NO_PROXY);
        }
        int port = p.getPort() >= 0 ? p.getPort() : "https".equals(p.getScheme()) ? 443 : 80;
        return List.of(new Proxy(Proxy.Type.HTTP, InetSocketAddress.createUnresolved(p.getHost(), port)));
    }

    @Override
    public void connectFailed(URI uri, SocketAddress sa, IOException ioe) {
        // Nothing to learn: the next call asks the same rule.
    }

    /** Whether {@code e} fits the {@code no_proxy} grammar. */
    static boolean validEntry(String e) {
        if (e.equals("*")) {
            return true;
        }
        int slash = e.indexOf('/');
        if (slash >= 0) {
            String bits = e.substring(slash + 1);
            byte[] ip = literal(e.substring(0, slash));
            if (ip == null || !bits.matches("[0-9]{1,3}")) {
                return false;
            }
            return Integer.parseInt(bits) <= ip.length * 8;
        }
        if (literal(Config.strip(e)) != null) {
            return true;
        }
        String host = e;
        int colon = e.lastIndexOf(':');
        if (colon >= 0 && (e.indexOf(':') == colon || e.substring(0, colon).endsWith("]"))) {
            host = e.substring(0, colon);
            String port = e.substring(colon + 1);
            if (!port.matches("[0-9]{1,5}") || Integer.parseInt(port) > 65535) {
                return false;
            }
        }
        while (host.startsWith(".")) {
            host = host.substring(1);
        }
        host = Config.strip(host);
        if (host.isEmpty()) {
            return false;
        }
        if (literal(host) != null) {
            return true;
        }
        for (String label : host.split("\\.", -1)) {
            if (label.isEmpty() || !label.matches("[A-Za-z0-9_-]+")) {
                return false;
            }
        }
        return true;
    }

    private static boolean inCidr(byte[] address, String cidr) {
        int slash = cidr.indexOf('/');
        byte[] net = literal(cidr.substring(0, slash));
        if (net == null || net.length != address.length) {
            return false;
        }
        int bits = Integer.parseInt(cidr.substring(slash + 1));
        for (int i = 0; i < net.length && bits > 0; i++, bits -= 8) {
            int mask = bits >= 8 ? 0xff : (0xff << (8 - bits)) & 0xff;
            if ((net[i] & mask) != (address[i] & mask)) {
                return false;
            }
        }
        return true;
    }

    /**
     * An IP literal's bytes, or {@code null} when {@code s} is not one. Never a DNS lookup: only
     * text that already reads as an address is parsed.
     */
    static byte[] literal(String s) {
        if (s.isEmpty()) {
            return null;
        }
        if (s.matches("[0-9]{1,3}(\\.[0-9]{1,3}){3}")) {
            String[] parts = s.split("\\.");
            byte[] out = new byte[4];
            for (int i = 0; i < 4; i++) {
                int v = Integer.parseInt(parts[i]);
                if (v > 255) {
                    return null;
                }
                out[i] = (byte) v;
            }
            return out;
        }
        if (s.contains(":") && s.matches("[0-9A-Fa-f:.]+")) {
            try {
                byte[] b = InetAddress.getByName(s).getAddress();
                if (b.length == 4) {
                    // An IPv4-mapped address reads as IPv4; keep the IPv6 form.
                    byte[] v6 = new byte[16];
                    v6[10] = (byte) 0xff;
                    v6[11] = (byte) 0xff;
                    System.arraycopy(b, 0, v6, 12, 4);
                    return v6;
                }
                return b;
            } catch (UnknownHostException e) {
                return null;
            }
        }
        return null;
    }
}
