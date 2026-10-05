"""The network: proxies and `no_proxy`, trust, client certificates, the user agent.

`docs/config.md` section 6. The `no_proxy` grammar is the SDK's own, the same in every
language, and is matched here rather than left to httpx.
"""

from __future__ import annotations

import hashlib
import ipaddress
import os
import platform
import ssl
import sys
from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import Any, cast
from urllib.parse import urlsplit

import httpx

from inorbithr._config import is_loopback
from inorbithr._errors import ConfigError
from inorbithr._version import SDK_VERSION

#: Where common Linux distributions keep the system's trust store.
_CA_FILES = (
    "/etc/ssl/cert.pem",
    "/etc/pki/tls/cert.pem",
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/ssl/ca-bundle.pem",
)


# --- the user agent ------------------------------------------------------------------


def _os_token() -> str:
    system = platform.system().lower()
    if hasattr(sys, "getandroidapilevel"):
        return "android"
    return {
        "linux": "linux",
        "darwin": "macos",
        "windows": "windows",
        "freebsd": "freebsd",
        "ios": "ios",
        "ipados": "ios",
    }.get(system, "other")


def _arch_token() -> str:
    machine = platform.machine().lower()
    return {
        "x86_64": "x86_64",
        "amd64": "x86_64",
        "aarch64": "aarch64",
        "arm64": "aarch64",
        "i386": "x86",
        "i686": "x86",
        "x86": "x86",
        "riscv64": "riscv64",
    }.get(machine, "arm" if machine.startswith("arm") else "other")


def user_agent(suffix: str | None) -> str:
    """`inorbithr-sdk-python/<v> python/<v> <os>/<arch>[ <suffix>]` (section 7.6)."""
    ua = (
        f"inorbithr-sdk-python/{SDK_VERSION} python/{platform.python_version()} "
        f"{_os_token()}/{_arch_token()}"
    )
    return f"{ua} {suffix}" if suffix else ua


# --- proxies -------------------------------------------------------------------------


def _strip_brackets(host: str) -> str:
    return host.removeprefix("[").removesuffix("]")


@dataclass(frozen=True)
class _Entry:
    any_host: bool = False
    network: ipaddress.IPv4Network | ipaddress.IPv6Network | None = None
    address: ipaddress.IPv4Address | ipaddress.IPv6Address | None = None
    domain: str = ""
    port: int | None = None


def _entry(e: str) -> _Entry:
    if e == "*":
        return _Entry(any_host=True)
    if "/" in e:
        try:
            return _Entry(network=ipaddress.ip_network(e, strict=False))
        except ValueError:
            raise ConfigError(f"{e!r} is not a no_proxy entry") from None
    try:
        return _Entry(address=ipaddress.ip_address(_strip_brackets(e)))
    except ValueError:
        pass
    host, port = e, None
    h, sep, p = e.rpartition(":")
    if sep and (":" not in h or h.endswith("]")):
        host, port = h, int(p)
    host = _strip_brackets(host.lstrip("."))
    try:
        return _Entry(address=ipaddress.ip_address(host), port=port)
    except ValueError:
        return _Entry(domain=host.lower(), port=port)


@dataclass(frozen=True)
class ProxyRule:
    """Which proxy, if any, a URL goes through (section 6.2)."""

    proxy: str | None = field(default=None, repr=False)
    """The proxy URL, user-info included; `None` for no proxy."""
    explicit: bool = False
    """Set in code, `INORBIT_PROXY` or the file: it applies to loopback too."""
    no_proxy: Sequence[str] = ()
    """The `no_proxy` entries, already checked."""

    @classmethod
    def from_settings(cls, values: dict[str, Any], sources: dict[str, Any]) -> ProxyRule:
        """The rule the resolved `proxy` and `no_proxy` settings give."""
        proxy = values.get("proxy")
        if proxy is None or proxy == "off":
            return cls(None)
        source = str(sources.get("proxy", {}).get("source", ""))
        explicit = source not in ("env https_proxy", "env HTTPS_PROXY")
        return cls(str(proxy), explicit, tuple(values.get("no_proxy") or ()))

    def proxy_for(self, url: str) -> str | None:  # noqa: PLR0911 - one return per no_proxy form
        """The proxy URL to use for `url`, or `None` for a direct connection."""
        if self.proxy is None:
            return None
        parts = urlsplit(url)
        host = (parts.hostname or "").lower()
        if not self.explicit and is_loopback(host):
            return None
        port = parts.port or (443 if parts.scheme == "https" else 80)
        try:
            address: ipaddress.IPv4Address | ipaddress.IPv6Address | None = ipaddress.ip_address(
                _strip_brackets(host)
            )
        except ValueError:
            address = None
        for raw in self.no_proxy:
            e = _entry(raw.strip().lower())
            if e.port is not None and e.port != port:
                continue
            if e.any_host:
                return None
            if e.network is not None:
                if (
                    address is not None
                    and address.version == e.network.version
                    and address in e.network
                ):
                    return None
                continue
            if e.address is not None:
                if address == e.address:
                    return None
                continue
            if e.domain and (host == e.domain or host.endswith("." + e.domain)):
                return None
        return self.proxy


# --- trust ---------------------------------------------------------------------------


def _spki_sha256(der: bytes) -> str:
    """The base64 SHA-256 of a DER certificate's SubjectPublicKeyInfo."""
    import base64  # noqa: PLC0415

    def header(buf: bytes, i: int) -> tuple[int, int, int]:
        tag = buf[i]
        n = buf[i + 1]
        i += 2
        if n & 0x80:
            k = n & 0x7F
            n = int.from_bytes(buf[i : i + k], "big")
            i += k
        return tag, i, n  # tag, content start, content length

    _, start, _ = header(der, 0)  # Certificate
    _, i, _ = header(der, start)  # TBSCertificate
    fields: list[tuple[int, int, int]] = []
    while len(fields) < 7:
        tag, s, n = header(der, i)
        fields.append((tag, i, s + n))
        i = s + n
    if fields[0][0] != 0xA0:  # no explicit version: v1, one field fewer before the key
        fields.insert(0, (0xA0, 0, 0))
    _, begin, end = fields[6]
    return base64.b64encode(hashlib.sha256(der[begin:end]).digest()).decode()


def _check_pins(peer: bytes | None, chain: Sequence[bytes], pins: frozenset[str]) -> None:
    candidates = [peer, *chain] if peer is not None else list(chain)
    if any(_spki_sha256(der) in pins for der in candidates):
        return
    raise ssl.SSLCertVerificationError("no certificate in the chain matches pinned_keys")


def _chain(obj: ssl.SSLSocket | ssl.SSLObject) -> list[bytes]:
    getter = getattr(obj, "get_verified_chain", None)  # Python 3.13 and later
    if getter is None:
        return []
    # DER bytes, the leaf first.
    return [bytes(c) for c in cast("list[Any]", getter()) if isinstance(c, bytes | bytearray)]


def _pinned_context(base: ssl.SSLContext, pins: Sequence[str]) -> ssl.SSLContext:
    """`base`, also refusing a handshake whose chain matches no pin (SR-06)."""
    wanted = frozenset(pins)

    class PinnedSocket(ssl.SSLSocket):
        def do_handshake(self, block: bool = False) -> None:
            super().do_handshake(block)
            _check_pins(self.getpeercert(binary_form=True), _chain(self), wanted)

    class PinnedObject(ssl.SSLObject):
        def do_handshake(self) -> None:
            super().do_handshake()
            _check_pins(self.getpeercert(binary_form=True), _chain(self), wanted)

    base.sslsocket_class = PinnedSocket
    base.sslobject_class = PinnedObject
    return base


def _system_store_missing() -> bool:
    if sys.platform in ("win32", "darwin"):
        return False
    paths = ssl.get_default_verify_paths()
    if paths.cafile and os.path.isfile(paths.cafile):
        return False
    if paths.capath and os.path.isdir(paths.capath) and any(os.scandir(paths.capath)):
        return False
    return not any(os.path.isfile(f) for f in _CA_FILES)


def ssl_context(
    *,
    ca_bundle: str | None = None,
    system_trust: bool = True,
    client_cert: str | None = None,
    client_key: str | None = None,
    client_key_password: str | None = None,
    pinned_keys: Sequence[str] = (),
) -> ssl.SSLContext:
    """The TLS context a client uses (sections 6.3 and 6.4).

    The system's trust store comes from `truststore` (the OS verifier on macOS and
    Windows, OpenSSL's system store elsewhere). `ca_bundle` adds to it; with
    `system_trust=False` only the bundle is trusted. Where a Linux host has no system
    store at all, the `certifi` bundle httpx already ships is used, as before.

    Raises:
        ConfigError: A file cannot be read or a key cannot be decrypted.
    """
    try:
        ctx: ssl.SSLContext
        if not system_trust:
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        elif pinned_keys:
            # Pinning hooks the handshake of a standard context; truststore's cannot be.
            ctx = ssl.create_default_context()
            if _system_store_missing():
                import certifi  # noqa: PLC0415 - httpx's own dependency

                ctx.load_verify_locations(certifi.where())
        elif _system_store_missing():
            import certifi  # noqa: PLC0415 - httpx's own dependency

            ctx = ssl.create_default_context(cafile=certifi.where())
        else:
            import truststore  # noqa: PLC0415 - only when a client is built

            ctx = truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        if ca_bundle is not None:
            ctx.load_verify_locations(cafile=ca_bundle)
        if client_cert is not None:
            ctx.load_cert_chain(client_cert, client_key, client_key_password)
    except (OSError, ssl.SSLError, ValueError) as e:
        raise ConfigError(f"the TLS settings are not usable: {type(e).__name__}") from None
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    ctx.verify_mode = ssl.CERT_REQUIRED
    ctx.check_hostname = True
    if pinned_keys:
        ctx = _pinned_context(ctx, pinned_keys)
    return ctx


# --- httpx ---------------------------------------------------------------------------


@dataclass(frozen=True)
class NetSettings:
    """What the HTTP client a client builds for itself needs."""

    connect_timeout: float = 10.0
    timeout: float = 30.0
    rule: ProxyRule = field(default_factory=ProxyRule)
    ca_bundle: str | None = None
    system_trust: bool = True
    client_cert: str | None = None
    client_key: str | None = None
    client_key_password: str | None = field(default=None, repr=False)
    pinned_keys: Sequence[str] = ()

    def context(self) -> ssl.SSLContext:
        """The TLS context for these settings."""
        return ssl_context(
            ca_bundle=self.ca_bundle,
            system_trust=self.system_trust,
            client_cert=self.client_cert,
            client_key=self.client_key,
            client_key_password=self.client_key_password,
            pinned_keys=self.pinned_keys,
        )

    def timeouts(self) -> httpx.Timeout:
        """Httpx's timeouts: `connect_timeout` for the connection, `timeout` for the rest."""
        return httpx.Timeout(self.timeout, connect=self.connect_timeout)


class _Router(httpx.BaseTransport):
    """Sends each request direct or through the proxy the rule picks."""

    def __init__(self, net: NetSettings, ctx: ssl.SSLContext) -> None:
        self._rule = net.rule
        self._direct = httpx.HTTPTransport(verify=ctx, trust_env=False)
        self._proxied = (
            httpx.HTTPTransport(verify=ctx, trust_env=False, proxy=httpx.Proxy(net.rule.proxy))
            if net.rule.proxy is not None
            else None
        )

    def handle_request(self, request: httpx.Request) -> httpx.Response:
        """Routes one request."""
        if self._proxied is not None and self._rule.proxy_for(str(request.url)) is not None:
            return self._proxied.handle_request(request)
        return self._direct.handle_request(request)

    def close(self) -> None:
        """Closes both connection pools."""
        self._direct.close()
        if self._proxied is not None:
            self._proxied.close()


class _AsyncRouter(httpx.AsyncBaseTransport):
    """`_Router` for asyncio."""

    def __init__(self, net: NetSettings, ctx: ssl.SSLContext) -> None:
        self._rule = net.rule
        self._direct = httpx.AsyncHTTPTransport(verify=ctx, trust_env=False)
        self._proxied = (
            httpx.AsyncHTTPTransport(verify=ctx, trust_env=False, proxy=httpx.Proxy(net.rule.proxy))
            if net.rule.proxy is not None
            else None
        )

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        """Routes one request."""
        if self._proxied is not None and self._rule.proxy_for(str(request.url)) is not None:
            return await self._proxied.handle_async_request(request)
        return await self._direct.handle_async_request(request)

    async def aclose(self) -> None:
        """Closes both connection pools."""
        await self._direct.aclose()
        if self._proxied is not None:
            await self._proxied.aclose()


def sync_http(net: NetSettings) -> tuple[httpx.Client, ssl.SSLContext]:
    """An `httpx.Client` for these settings, and its TLS context (for the socket)."""
    ctx = net.context()
    client = httpx.Client(
        transport=_Router(net, ctx),
        trust_env=False,
        follow_redirects=False,
        timeout=net.timeouts(),
    )
    return client, ctx


def async_http(net: NetSettings) -> tuple[httpx.AsyncClient, ssl.SSLContext]:
    """An `httpx.AsyncClient` for these settings, and its TLS context (for the socket)."""
    ctx = net.context()
    client = httpx.AsyncClient(
        transport=_AsyncRouter(net, ctx),
        trust_env=False,
        follow_redirects=False,
        timeout=net.timeouts(),
    )
    return client, ctx
