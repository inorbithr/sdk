"""Configuration resolution: code, the environment, the config file, defaults.

`docs/config.md` sections 2 to 5 are the contract; `cli/crates/iohr/src/sdk_config.rs` is
the reference implementation it was written against. Resolution is pure: the
environment, the OS, the home and working directories and the file reader are inputs
(`LoadOptions`), so the conformance vectors run without touching the process.
"""

from __future__ import annotations

import base64
import binascii
import ipaddress
import os
import platform
import shutil
import tomllib
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from datetime import timedelta
from typing import Any, Literal, TypeAlias, cast
from urllib.parse import urlsplit

from inorbithr._errors import ConfigError, ConfigProblem

#: The built-in pipeline, outermost first (config.md section 7.2).
PIPELINE: tuple[str, ...] = (
    "request_id",
    "user_agent",
    "idempotency_key",
    "call_tracing",
    "deadline",
    "retry",
    "auth",
    "rate_limit",
    "attempt_tracing",
    "logging",
    "hooks",
    "timeout",
)

#: The largest config file read (section 4.1).
MAX_FILE = 1024 * 1024
REDACTED = "<redacted>"
#: The keys of a profile table the command line owns (section 4.2).
CLI_KEYS = frozenset({"kind", "account", "storage", "issuer", "client_id"})
#: The credential sources `credential_sources` may name, in chain order.
SOURCES: tuple[str, ...] = ("env", "workload", "file", "cli")

#: An operating system whose conventions apply to paths.
OsName: TypeAlias = Literal["linux", "macos", "windows"]


def current_os() -> OsName:
    """The OS this process runs on, by the conventions that matter for paths."""
    system = platform.system()
    if system == "Darwin":
        return "macos"
    if system == "Windows":
        return "windows"
    return "linux"


@dataclass(frozen=True)
class LoadOptions:
    """What `load` reads instead of the process, for tests and tools.

    Every field left `None` is the process's own: `os.environ`, the running OS, the
    user's home directory, the working directory.
    """

    env: Mapping[str, str] | None = None
    """The environment `load` sees; an empty value is unset."""
    os: OsName | None = None
    """`linux`, `macos` or `windows`: where the config file is looked for."""
    home: str | None = None
    """The home directory; `""` means there is none (a sandbox, a distroless container)."""
    cwd: str | None = None
    """The working directory, for relative paths in code and the environment."""


# --- paths ---------------------------------------------------------------------------


def _sep(os_name: OsName) -> str:
    return "\\" if os_name == "windows" else "/"


def _is_absolute(os_name: OsName, p: str) -> bool:
    if os.path.isabs(p):
        return True
    if os_name == "windows":
        return p.startswith(("\\", "/")) or (
            len(p) >= 3 and p[0].isascii() and p[0].isalpha() and p[1] == ":" and p[2] in "\\/"
        )
    return p.startswith("/")


def _join(os_name: OsName, directory: str, rest: str) -> str:
    return directory.rstrip("/\\") + _sep(os_name) + rest


def _parent(os_name: OsName, path: str) -> str:
    i = max(path.rfind("/"), path.rfind("\\"))
    parent = "." if i < 0 else path[:1] if i == 0 else path[:i]
    other = "/" if os_name == "windows" else "\\"
    return parent.replace(other, _sep(os_name))


def config_path(  # noqa: PLR0911 - one return per rule of section 4.1
    os_name: OsName,
    env: Mapping[str, str],
    home: str | None,
    code: str | None = None,
) -> tuple[str, bool, str] | None:
    """Where the config file is (section 4.1): `None` reads no file.

    Args:
        os_name: The OS whose default location applies.
        env: The environment; an empty value is unset.
        home: The home directory, or `None` when there is none.
        code: `config_file` set in code, if any.

    Returns:
        The path, whether it was named (by code or `INORBIT_CONFIG_FILE`, so a missing
        file is an error), and the label of where it came from; or `None`.
    """

    def var(k: str) -> str | None:
        return env.get(k) or None

    if code is not None:
        return None if code == "off" else (code, True, "code")
    named = var("INORBIT_CONFIG_FILE")
    if named is not None:
        return None if named == "off" else (named, True, "env INORBIT_CONFIG_FILE")
    directory = var("IOHR_CONFIG_DIR")
    if directory is not None:
        return (_join(os_name, directory, "config.toml"), False, "env IOHR_CONFIG_DIR")
    if os_name == "linux":
        xdg = var("XDG_CONFIG_HOME")
        if xdg is not None and _is_absolute(os_name, xdg):
            return (_join(os_name, xdg, "iohr/config.toml"), False, "default")
        if home:
            return (_join(os_name, home, ".config/iohr/config.toml"), False, "default")
        return None
    if os_name == "macos":
        if not home:
            return None
        rest = "Library/Application Support/hr.InOrbit.iohr/config.toml"
        return (_join(os_name, home, rest), False, "default")
    appdata = var("APPDATA")
    if appdata is None:
        return None
    return (_join(os_name, appdata, "InOrbit\\iohr\\config\\config.toml"), False, "default")


# --- value syntax --------------------------------------------------------------------


def parse_duration(value: str) -> int | None:
    """A duration string in milliseconds: digits, then `ms`, `s`, `m` or `h`, above zero.

    Args:
        value: The text, such as `30s`.

    Returns:
        The milliseconds, or `None` when the text is not a duration.
    """
    i = 0
    while i < len(value) and value[i].isascii() and value[i].isdigit():
        i += 1
    if i == 0 or i == len(value):
        return None
    n = int(value[:i])
    scale = {"ms": 1, "s": 1000, "m": 60_000, "h": 3_600_000}.get(value[i:])
    if scale is None:
        return None
    ms = n * scale
    return ms if ms > 0 else None


def show_duration(ms: int) -> str:
    """A duration as `describe()` shows it: whole seconds as `30s`, else `500ms`."""
    return f"{ms // 1000}s" if ms % 1000 == 0 else f"{ms}ms"


def is_loopback(host: str) -> bool:
    """Whether `host` is `localhost` or a loopback address (brackets allowed)."""
    h = host.removeprefix("[").removesuffix("]")
    if h.lower() == "localhost":
        return True
    try:
        return ipaddress.ip_address(h).is_loopback
    except ValueError:
        return False


def redact_userinfo(raw: str) -> str:
    """A URL with its user-info replaced by `<redacted>`."""
    scheme, sep, rest = raw.partition("://")
    if not sep:
        return raw
    end = len(rest)
    for c in "/?#":
        i = rest.find(c)
        if 0 <= i < end:
            end = i
    at = rest[:end].rfind("@")
    return raw if at < 0 else f"{scheme}://{REDACTED}@{rest[at + 1 :]}"


def _has_userinfo(raw: str) -> bool:
    try:
        u = urlsplit(raw)
    except ValueError:
        return False
    return bool(u.username) or u.password is not None


def _absolute_url(raw: str) -> bool:
    try:
        u = urlsplit(raw)
        _ = u.port
    except ValueError:
        return False
    return bool(u.scheme) and bool(u.hostname)


def no_proxy_entry(e: str) -> bool:  # noqa: PLR0911 - one return per form of the grammar
    """Whether `e` fits the `no_proxy` grammar (section 6.2)."""
    if e == "*":
        return True
    if "/" in e:
        ip, _, bits = e.partition("/")
        if not bits.isdigit():
            return False
        try:
            addr = ipaddress.ip_address(ip)
        except ValueError:
            return False
        return int(bits) <= (32 if addr.version == 4 else 128)
    bare = e.removeprefix("[").removesuffix("]")
    try:
        ipaddress.ip_address(bare)
    except ValueError:
        pass
    else:
        return True
    host, port = e, None
    h, sep, p = e.rpartition(":")
    if sep and (":" not in h or h.endswith("]")):
        host, port = h, p
    if port is not None and not (port.isdigit() and int(port) <= 65535):
        return False
    host = host.lstrip(".").removeprefix("[").removesuffix("]")
    if not host:
        return False
    try:
        ipaddress.ip_address(host)
    except ValueError:
        pass
    else:
        return True
    return all(
        label and all(c.isascii() and (c.isalnum() or c in "-_") for c in label)
        for label in host.split(".")
    )


def env_name(profile: str) -> str:
    """A profile's environment form: upper case, `-` as `_` (`acme-ci` → `ACME_CI`)."""
    return profile.upper().replace("-", "_")


def valid_profile(name: str) -> bool:
    """A profile name the command line accepts (section 2.3)."""
    return (
        1 <= len(name) <= 64
        and all(c.isascii() and (c.islower() or c.isdigit() or c in "-_") for c in name)
        and (name[0].islower() or name[0].isdigit())
    )


# --- the catalogue -------------------------------------------------------------------

_Ty: TypeAlias = Literal[
    "duration",
    "int",
    "bool",
    "scopes",
    "list",
    "url",
    "path",
    "secret",
    "str",
    "enum",
    "proxy",
    "pins",
    "reserved",
]


@dataclass(frozen=True)
class _Setting:
    name: str
    ty: _Ty
    in_file: bool = True
    credential: bool = False
    transport: bool = False
    default: object = None
    choices: tuple[str, ...] = ()


def _d(name: str, ty: _Ty, default: object, **kw: Any) -> _Setting:  # noqa: ANN401
    return _Setting(name, ty, default=default, **kw)


#: The catalogue (section 3), in its order; problems are reported in this order.
CATALOGUE: tuple[_Setting, ...] = (
    _d("base_url", "url", "https://api.inorbit.hr"),
    _d("token_url", "url", "https://auth.inorbit.hr/oauth2/token"),
    _Setting("region", "reserved"),
    _Setting("key_id", "str", credential=True),
    _Setting("key_secret", "secret", in_file=False, credential=True),
    _Setting("key_secret_file", "path", credential=True),
    _Setting("scopes", "scopes"),
    _Setting("token", "secret", in_file=False, credential=True),
    _Setting("token_file", "path", credential=True),
    _d("credential_sources", "list", list(SOURCES)),
    _Setting("cli_path", "path"),
    _d("connect_timeout", "duration", "10s", transport=True),
    _d("timeout", "duration", "30s"),
    _d("total_timeout", "duration", "120s"),
    _d("stream_idle_timeout", "duration", "45s"),
    _d("max_retries", "int", 2),
    _d("retry_base_delay", "duration", "500ms"),
    _d("retry_max_delay", "duration", "8s"),
    _d("retry_after_max", "duration", "60s"),
    _d("retry_budget", "bool", True),
    _d("streams", "enum", "sse", choices=("sse", "socket")),
    _Setting("proxy", "proxy", transport=True),
    _Setting("no_proxy", "list", transport=True),
    _Setting("ca_bundle", "path", transport=True),
    _d("system_trust", "bool", True, transport=True),
    _Setting("client_cert", "path", transport=True),
    _Setting("client_key", "path", transport=True),
    _Setting("client_key_password", "secret", in_file=False, transport=True),
    _Setting("pinned_keys", "pins", transport=True),
    _d("log", "enum", "off", choices=("off", "error", "warn", "info", "debug")),
    _d("log_headers", "bool", False),
    _Setting("log_allow_headers", "list"),
    _Setting("tracing", "bool"),
    _Setting("metrics", "bool"),
    _d("rate_limit", "enum", "observe", choices=("observe", "wait", "off")),
    _Setting("user_agent_suffix", "str"),
)
_BY_NAME = {s.name: s for s in CATALOGUE}

#: Options that only code can set: they hold objects (section 3.5).
CODE_ONLY = frozenset(
    {
        "token_provider",
        "http_client",
        "pipeline",
        "hooks",
        "logger",
        "redact",
        "tracer_provider",
        "meter_provider",
        "retry_budget_capacity",
        "profile",
        "config_file",
        "profile_type",
    }
)


def _order(setting: str) -> int:
    if setting == "profile":
        return 0
    if setting == "config_file":
        return 1
    if setting == "credential":
        return 1_000_001
    names = [s.name for s in CATALOGUE]
    return names.index(setting) + 2 if setting in names else 1_000_000


def _toml_type(v: object) -> str:  # noqa: PLR0911 - one return per TOML type
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, int):
        return "integer"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "string"
    if isinstance(v, list):
        return "array"
    if isinstance(v, dict):
        return "table"
    return "datetime"


def _q(v: str) -> str:
    """A value quoted as the reference resolver quotes it."""
    return '"' + v.replace("\\", "\\\\").replace('"', '\\"') + '"'


@dataclass(frozen=True)
class _Raw:
    origin: Literal["code", "env", "file"]
    value: object


@dataclass
class Credential:
    """The credential `load` chose: its source and kind, and what builds it.

    Holds secrets; never printed (its `repr` names the source and kind only).
    """

    source: str
    kind: str
    token: str | None = field(default=None, repr=False)
    token_file: str | None = None
    key_id: str | None = None
    key_secret: str | None = field(default=None, repr=False)
    key_secret_file: str | None = None
    scopes: list[str] = field(default_factory=list[str])
    cli_path: str = "iohr"
    profile: str | None = None
    provider: object = field(default=None, repr=False)


@dataclass
class Resolution:
    """What resolution found: the `describe()` document, the typed values, the credential."""

    doc: dict[str, Any]
    values: dict[str, Any]
    credential: Credential | None
    file_path: str | None


class ResolvedConfig:
    """A client's effective configuration and where each value came from (section 2.6)."""

    def __init__(self, doc: dict[str, Any]) -> None:
        """The configuration as `describe()` shows it."""
        self._doc = doc

    def describe(self) -> dict[str, Any]:
        """The effective configuration as JSON-ready data.

        The profile, the config file, each setting with its source, the credential chain,
        the pipeline and what was ignored. Secrets are always `<redacted>`.
        """
        import copy  # noqa: PLC0415 - only when asked

        return copy.deepcopy(self._doc)

    def to_json(self) -> str:
        """`describe()` as indented JSON text."""
        import json  # noqa: PLC0415 - only when asked

        return json.dumps(self._doc, indent=2)

    def __repr__(self) -> str:
        """The document; it holds no secret."""
        return f"ResolvedConfig({self._doc!r})"


def _default_read(path: str) -> bytes | None:
    try:
        with open(path, "rb") as f:
            return f.read(MAX_FILE + 1)
    except OSError:
        return None


def cli_found(program: str, env: Mapping[str, str]) -> bool:
    """Whether `program` can be run: a path to a file, or a name found on `PATH`."""
    if os.sep in program or (os.altsep and os.altsep in program) or os.path.isabs(program):
        return os.path.isfile(program)
    path = env.get("PATH")
    if not path:
        return False
    return shutil.which(program, path=path) is not None


class _Resolver:
    def __init__(  # noqa: PLR0917 - internal, built once per resolution
        self,
        code: Mapping[str, object],
        env: Mapping[str, str],
        os_name: OsName,
        home: str | None,
        cwd: str,
        profile_type: str | None,
        found: Callable[[str], bool],
        read: Callable[[str], bytes | None],
    ) -> None:
        self.code = {k: v for k, v in code.items() if v is not None}
        self.env = env
        self.os: OsName = os_name
        self.home = home
        self.cwd = cwd
        self.profile_type = profile_type
        self.found = found
        self.read = read
        self.prefix = f"INORBIT_{env_name(profile_type)}_" if profile_type else None
        self.problems: list[ConfigProblem] = []
        self.settings: dict[str, dict[str, Any]] = {}
        self.values: dict[str, Any] = {}
        self.ignored: list[dict[str, Any]] = []
        self.layers: list[tuple[dict[str, Any], str, bool]] = []
        self.file_path: str | None = None
        self.file_dir = cwd
        self.tried: list[dict[str, str]] = []

    def var(self, k: str) -> str | None:
        return self.env.get(k) or None

    def problem(self, setting: str, source: str, message: str) -> None:
        self.problems.append(ConfigProblem(setting, source, message))

    def http_client(self) -> bool:
        return "http_client" in self.code

    # The file and the profile.

    def file_label(self, suffix: str = "") -> str:
        if self.file_path is None:
            return ""
        return f"file {self.file_path}" + (f" [{suffix}]" if suffix else "")

    def load_file(self) -> dict[str, Any]:
        code_file = self.code.get("config_file")
        located = config_path(
            self.os,
            self.env,
            self.home,
            os.fspath(cast("str", code_file)) if code_file is not None else None,
        )
        if located is None:
            return {}
        path, named, label = located
        p = path if _is_absolute(self.os, path) else _join(self.os, self.cwd, path)
        data = self.read(p)
        if data is None:
            if named:
                self.problem("config_file", label, f"there is no readable file at {p}")
            return {}
        if len(data) > MAX_FILE:
            self.problem("config_file", label, f"{p} is larger than 1 MiB")
            return {}
        try:
            doc = tomllib.loads(data.decode("utf-8"))
        except UnicodeDecodeError:
            self.problem("config_file", label, f"{p} is not valid TOML: it is not UTF-8")
            return {}
        except tomllib.TOMLDecodeError as e:
            # The message only: a snippet of the file could quote a secret.
            self.problem("config_file", label, f"{p} is not valid TOML: {e}")
            return {}
        self.file_path = p
        self.file_dir = _parent(self.os, p)
        return doc

    def choose_profile(self, doc: dict[str, Any]) -> tuple[str, str] | None:
        if self.profile_type is not None:
            return (self.profile_type, "code")
        chosen: tuple[str, str] | None = None
        code_profile = self.code.get("profile")
        if isinstance(code_profile, str):
            chosen = (code_profile, "code")
        elif (v := self.var("INORBIT_PROFILE")) is not None:
            chosen = (v, "env INORBIT_PROFILE")
        elif isinstance(doc.get("default"), str):
            chosen = (cast("str", doc["default"]), self.file_label())
        if chosen is None:
            return None
        name, source = chosen
        profiles = doc.get("profiles")
        has = isinstance(profiles, dict) and name in profiles
        if not valid_profile(name):
            self.problem(
                "profile",
                source,
                f"{_q(name)} is not a profile name: 1 to 64 lower-case letters, digits, "
                "'-' or '_', starting with a letter or digit",
            )
            return None
        if not has:
            where = (
                "no config file was read"
                if self.file_path is None
                else f"{self.file_path} has no [profiles.{name}]"
            )
            self.problem(
                "profile",
                source,
                f"there is no profile {_q(name)}: {where}; `iohr profile list` shows the profiles",
            )
            return None
        return chosen

    def check_file_keys(self) -> None:
        for table, label, is_profile in self.layers:
            for k, v in table.items():
                if is_profile and k in CLI_KEYS:
                    continue
                s = _BY_NAME.get(k)
                if s is not None and not s.in_file:
                    way_out = {
                        "key_secret": "key_secret_file, the environment, or iohr login",
                        "token": "token_file, the environment, or iohr login",
                    }.get(s.name, "the environment or code")
                    self.problem(
                        k, label, f"secrets are not allowed in the config file; use {way_out}"
                    )
                elif s is not None and s.ty == "proxy" and isinstance(v, str) and _has_userinfo(v):
                    self.problem(
                        k,
                        label,
                        "a proxy URL with a user name or password holds a secret, which is not "
                        "allowed in the config file; set it in INORBIT_PROXY or in code",
                    )
                elif s is None and k in ("profile", "config_file"):
                    self.ignored.append(
                        {"key": k, "source": label, "reason": "not read from the config file"}
                    )
                elif s is None:
                    self.ignored.append({"key": k, "source": label, "reason": "unknown key"})

    # Settings.

    def env_names(self, s: _Setting) -> list[str]:
        upper = s.name.upper()
        names: list[str] = []
        if self.prefix is not None:
            names.append(f"{self.prefix}{upper}")
        if not (s.credential and self.prefix is not None):
            names.append(f"INORBIT_{upper}")
        return names

    def raw(self, s: _Setting) -> tuple[_Raw, str] | None:
        if s.name in self.code:
            return (_Raw("code", self.code[s.name]), "code")
        for n in self.env_names(s):
            v = self.var(n)
            if v is not None:
                return (_Raw("env", v), f"env {n}")
        if s.in_file:
            for table, label, _ in self.layers:
                if s.name in table:
                    return (_Raw("file", table[s.name]), label)
        after = {"proxy": ("https_proxy", "HTTPS_PROXY"), "no_proxy": ("no_proxy", "NO_PROXY")}
        for n in after.get(s.name, ()):
            v = self.var(n)
            if v is not None:
                return (_Raw("env", v), f"env {n}")
        return None

    def show(self, name: str, shown: object, source: str, value: object = None) -> None:
        self.settings[name] = {"value": shown, "source": source}
        self.values[name] = shown if value is None else value

    def resolve_settings(self) -> None:
        for s in CATALOGUE:
            if s.credential:
                continue
            found = self.raw(s)
            if found is None:
                if s.default is not None and not (s.transport and self.http_client()):
                    shown, value = self.parse(s, _Raw("code", s.default))
                    self.show(s.name, shown, "default", value)
                continue
            raw, source = found
            if s.transport and self.http_client():
                if source == "code":
                    self.problem(
                        s.name,
                        source,
                        "configure this on your HTTP client, or leave http_client out",
                    )
                else:
                    self.ignored.append(
                        {
                            "key": s.name,
                            "source": source,
                            "reason": "the caller's HTTP client decides this",
                        }
                    )
                continue
            try:
                shown, value = self.parse(s, raw)
            except ValueError as e:
                self.problem(s.name, source, str(e))
                continue
            self.show(s.name, shown, source, value)

    def path(self, p: str, from_file: bool) -> str:
        if p.startswith("~/"):
            if not self.home:
                raise ValueError(f"{p} starts with ~/ but there is no home directory")
            return _join(self.os, self.home, p[2:])
        if _is_absolute(self.os, p):
            return p
        return _join(self.os, self.file_dir if from_file else self.cwd, p)

    def parse(self, s: _Setting, raw: _Raw) -> tuple[object, object]:  # noqa: PLR0911, PLR0912, PLR0915
        """The value as `describe()` shows it, and as the client uses it."""
        v = raw.value
        origin = raw.origin

        def text(what: str) -> str:
            if isinstance(v, str):
                return v
            if s.ty == "path" and isinstance(v, os.PathLike):
                return os.fspath(cast("os.PathLike[str]", v))
            if origin == "file":
                raise ValueError(f"must be {what}, not {_toml_type(raw.value)}")
            raise ValueError(f"must be {what}")

        def items(comma: bool) -> list[str]:
            if isinstance(v, str) and origin in ("env", "code"):
                if comma:
                    return [x.strip() for x in v.split(",") if x.strip()]
                return v.split()
            if isinstance(v, list | tuple):
                out: list[str] = []
                for x in cast("Sequence[object]", v):
                    if not isinstance(x, str):
                        raise ValueError(  # noqa: TRY004 - reported as a ConfigProblem, not a programming error
                            "must be an array of strings"
                            if origin == "file"
                            else "must be a list of strings"
                        )
                    out.append(x)
                return out
            if origin == "file":
                raise ValueError(f"must be an array of strings, not {_toml_type(v)}")
            raise ValueError("must be a list of strings")

        if s.ty == "duration":
            ms: int | None
            if origin == "code" and isinstance(v, timedelta):
                ms = round(v.total_seconds() * 1000)
                ms = ms if ms > 0 else None
            elif origin == "code" and isinstance(v, int | float) and not isinstance(v, bool):
                ms = round(float(v) * 1000)
                ms = ms if ms > 0 else None
            else:
                t = text('a duration string such as "30s"')
                ms = parse_duration(t)
                if ms is None:
                    if parse_duration(t + "s") is not None:
                        raise ValueError(
                            f"{_q(t)} is not a duration; write it with a unit, such as 30s"
                        )
                    raise ValueError(
                        f"{_q(t)} is not a duration greater than zero: digits, then ms, s, m "
                        "or h, such as 30s"
                    )
            if ms is None:
                raise ValueError("must be a duration greater than zero")
            return show_duration(ms), ms / 1000
        if s.ty == "int":
            if origin == "env":
                t = cast("str", v)
                if not (t.isascii() and t.isdigit()) or int(t) > 0xFFFFFFFF:
                    raise ValueError(f"{_q(t)} is not a whole number of 0 or more")
                return int(t), int(t)
            if isinstance(v, bool) or not isinstance(v, int):
                raise ValueError(
                    f"must be an integer, not {_toml_type(v)}"
                    if origin == "file"
                    else "must be an integer"
                )
            if not 0 <= v <= 0xFFFFFFFF:
                raise ValueError("must be a whole number of 0 or more")
            return v, v
        if s.ty == "bool":
            if origin == "env":
                t = cast("str", v).lower()
                if t in ("true", "1"):
                    return True, True
                if t in ("false", "0"):
                    return False, False
                raise ValueError(f"{_q(cast('str', v))} is not true, false, 1 or 0")
            if isinstance(v, bool):
                return v, v
            raise ValueError(
                f"must be a boolean, not {_toml_type(v)}"
                if origin == "file"
                else "must be a boolean"
            )
        if s.ty == "scopes":
            got = items(comma=False)
            return got, got
        if s.ty == "list":
            got = items(comma=True)
            if s.name == "credential_sources":
                for x in got:
                    if x not in SOURCES:
                        raise ValueError(
                            f"{_q(x)} is not a credential source; use env, workload, file or cli"
                        )
            if s.name == "no_proxy":
                for x in got:
                    if not no_proxy_entry(x):
                        raise ValueError(
                            f"{_q(x)} is not a no_proxy entry: a host, .domain, host:port, an IP "
                            "address or a CIDR range"
                        )
            if s.name == "log_allow_headers":
                got = [x.lower() for x in got]
            return got, got
        if s.ty == "url":
            t = text("a URL string")
            if not _absolute_url(t):
                raise ValueError(f"{_q(t)} is not an absolute URL")
            u = urlsplit(t)
            if not (u.scheme == "https" or (u.scheme == "http" and is_loopback(u.hostname or ""))):
                raise ValueError(
                    f"{_q(t)} must use https (plain http is allowed only for localhost and "
                    "loopback addresses)"
                )
            return t, t
        if s.ty == "path":
            t = text("a path string")
            p = self.path(t, from_file=origin == "file")
            return p, p
        if s.ty == "secret":
            if isinstance(v, str):
                return REDACTED, v
            raise ValueError("must be a string")
        if s.ty == "str":
            t = text("a string")
            if s.name == "user_agent_suffix" and (
                len(t) > 128 or any(not (c.isascii() and (c.isprintable())) for c in t)
            ):
                raise ValueError(
                    "must be product tokens (such as myapp/1.2), at most 128 printable ASCII "
                    "characters"
                )
            return t, t
        if s.ty == "enum":
            t = text("a string")
            if t not in s.choices:
                raise ValueError(f"{_q(t)} is not one of {', '.join(s.choices)}")
            return t, t
        if s.ty == "proxy":
            t = text("a URL string")
            if t == "off":
                return "off", "off"
            shown = redact_userinfo(t)
            if not _absolute_url(t):
                raise ValueError(f"{_q(shown)} is not an absolute URL")
            if urlsplit(t).scheme not in ("http", "https"):
                raise ValueError(f"{_q(shown)} must be an http:// or https:// proxy URL, or off")
            return shown, t
        if s.ty == "pins":
            got = items(comma=True)
            if len(got) < 2:
                raise ValueError("pin at least two keys (the current one and a backup)")
            for p in got:
                try:
                    ok = len(base64.b64decode(p, validate=True)) == 32
                except (binascii.Error, ValueError):
                    ok = False
                if not ok:
                    raise ValueError(f"{_q(p)} is not a base64 SHA-256 of a public key")
            return got, got
        raise ValueError("region is reserved until the API offers regions; remove it")

    def exists(self, path: str) -> bool:
        return self.read(path) is not None

    def allowed(self, source: str) -> bool:
        got = self.values.get("credential_sources")
        return got is None or source in got

    def read_text(self, path: str) -> str:
        data = self.read(path)
        return "" if data is None else data.decode("utf-8", errors="replace").strip()

    # The chain.

    def chain(  # noqa: PLR0911, PLR0912, PLR0915
        self, profile: str | None, table: dict[str, Any] | None
    ) -> Credential | None:
        tried: list[dict[str, str]] = []

        def skip(source: str, reason: str) -> None:
            tried.append({"source": source, "result": "skipped", "reason": reason})

        p = self.prefix or "INORBIT_"
        used: Credential | None = None
        scopes: list[str] = list(self.values.get("scopes") or [])

        # 1. Code.
        code = self.code
        if "token_provider" in code:
            used = Credential("code", "custom", provider=code["token_provider"])
        elif "token" in code:
            used = Credential("code", "static_token", token=str(code["token"]))
            self.show("token", REDACTED, "code", code["token"])
        elif "key_id" in code:
            code_key = str(code["key_id"])
            code_secret, code_secret_file = code.get("key_secret"), code.get("key_secret_file")
            if code_secret is not None and code_secret_file is not None:
                self.problem(
                    "key_secret", "code", "key_secret and key_secret_file are both set; set one"
                )
                return None
            if code_secret is None and code_secret_file is None:
                self.problem(
                    "key_secret", "code", "key_id is set without key_secret or key_secret_file"
                )
                return None
            if not scopes:
                self.problem("scopes", "code", "a key needs scopes: set scopes")
                return None
            used = Credential("code", "client_credentials", key_id=code_key, scopes=scopes)
            self.show("key_id", code_key, "code")
            if code_secret is not None:
                used.key_secret = str(code_secret)
                self.show("key_secret", REDACTED, "code", code_secret)
            else:
                path = self.path(os.fspath(cast("str", code_secret_file)), from_file=False)
                if not self.exists(path):
                    self.problem("key_secret_file", "code", f"cannot read {path}")
                    return None
                used.key_secret_file = path
                self.show("key_secret_file", path, "code")
        elif "token_file" in code:
            path = self.path(os.fspath(cast("str", code["token_file"])), from_file=False)
            if not self.exists(path):
                self.problem("token_file", "code", f"cannot read {path}")
                return None
            used = Credential("code", "token_file", token_file=path)
            self.show("token_file", path, "code")
        if used is not None:
            tried.append({"source": "code", "result": "used"})
        else:
            skip("code", "none set")

        # 2. The environment.
        if used is None:

            def n(x: str) -> str:
                return f"{p}{x}"

            def src(x: str) -> str:
                return f"env {p}{x}"

            token = self.var(n("TOKEN"))
            token_file = self.var(n("TOKEN_FILE"))
            key_id = self.var(n("KEY_ID"))
            secret = self.var(n("KEY_SECRET"))
            secret_file = self.var(n("KEY_SECRET_FILE"))
            if not self.allowed("env"):
                skip("env", "not in credential_sources")
            elif token is None and token_file is None and key_id is None:
                skip("env", f"{n('TOKEN')}, {n('TOKEN_FILE')} and {n('KEY_ID')} are not set")
            else:
                forms = [
                    x
                    for x, here in (
                        ("TOKEN", token),
                        ("TOKEN_FILE", token_file),
                        ("KEY_ID", key_id),
                    )
                    if here is not None
                ]
                if len(forms) > 1:
                    self.problem(
                        forms[0].lower(),
                        src(forms[0]),
                        f"{' and '.join(n(x) for x in forms)} are both set; set one credential",
                    )
                    return None
                if token is not None:
                    self.show("token", REDACTED, src("TOKEN"), token)
                    used = Credential("env", "static_token", token=token)
                elif token_file is not None:
                    path = self.path(token_file, from_file=False)
                    if not self.exists(path):
                        self.problem("token_file", src("TOKEN_FILE"), f"cannot read {path}")
                        return None
                    self.show("token_file", path, src("TOKEN_FILE"))
                    used = Credential("env", "token_file", token_file=path)
                elif key_id is not None:
                    if secret is not None and secret_file is not None:
                        self.problem(
                            "key_secret",
                            src("KEY_SECRET"),
                            f"{n('KEY_SECRET')} and {n('KEY_SECRET_FILE')} are both set; set one",
                        )
                        return None
                    if secret is None and secret_file is None:
                        self.problem(
                            "key_secret",
                            src("KEY_ID"),
                            f"{n('KEY_ID')} is set without {n('KEY_SECRET')} or "
                            f"{n('KEY_SECRET_FILE')}",
                        )
                        return None
                    if not scopes:
                        self.problem("scopes", src("KEY_ID"), f"a key needs scopes: set {p}SCOPES")
                        return None
                    self.show("key_id", key_id, src("KEY_ID"))
                    used = Credential("env", "client_credentials", key_id=key_id, scopes=scopes)
                    if secret is not None:
                        self.show("key_secret", REDACTED, src("KEY_SECRET"), secret)
                        used.key_secret = secret
                    elif secret_file is not None:
                        path = self.path(secret_file, from_file=False)
                        if not self.exists(path):
                            self.problem(
                                "key_secret_file", src("KEY_SECRET_FILE"), f"cannot read {path}"
                            )
                            return None
                        self.show("key_secret_file", path, src("KEY_SECRET_FILE"))
                        used.key_secret_file = path
                if used is not None:
                    tried.append({"source": "env", "result": "used"})

        # 3. Workload identity: reserved until the platform offers it (section 5.5).
        if used is None:
            skip(
                "workload",
                "not offered by the platform yet"
                if self.allowed("workload")
                else "not in credential_sources",
            )

        # 4. The config file's profile table.
        if used is None:
            label = self.layers[0][1] if self.layers else ""
            if not self.allowed("file"):
                skip("file", "not in credential_sources")
            elif self.file_path is None:
                skip("file", "no config file was read")
            elif profile is None:
                skip("file", "no profile chosen")
            elif table is not None and ("token_file" in table or "key_id" in table):
                if "token_file" in table and "key_id" in table:
                    self.problem(
                        "token_file",
                        label,
                        "token_file and key_id are both set; set one credential",
                    )
                    return None
                tf, kid = table.get("token_file"), table.get("key_id")
                if isinstance(tf, str):
                    try:
                        path = self.path(tf, from_file=True)
                    except ValueError as e:
                        self.problem("token_file", label, str(e))
                        return None
                    if not self.exists(path):
                        self.problem("token_file", label, f"cannot read {path}")
                        return None
                    self.show("token_file", path, label)
                    used = Credential("file", "token_file", token_file=path)
                elif isinstance(kid, str):
                    if "key_secret" in table:
                        return None  # Already reported: secrets are not allowed in the file.
                    sf = table.get("key_secret_file")
                    if not isinstance(sf, str):
                        self.problem("key_secret", label, "key_id is set without key_secret_file")
                        return None
                    if not scopes:
                        self.problem(
                            "scopes", label, "a key needs scopes: set scopes in the profile's table"
                        )
                        return None
                    try:
                        path = self.path(sf, from_file=True)
                    except ValueError as e:
                        self.problem("key_secret_file", label, str(e))
                        return None
                    if not self.exists(path):
                        self.problem("key_secret_file", label, f"cannot read {path}")
                        return None
                    self.show("key_id", kid, label)
                    self.show("key_secret_file", path, label)
                    used = Credential(
                        "file",
                        "client_credentials",
                        key_id=kid,
                        key_secret_file=path,
                        scopes=scopes,
                    )
                else:
                    self.problem("token_file", label, "must be a path string")
                    return None
                tried.append({"source": "file", "result": "used"})
            else:
                skip("file", f"profile {profile} sets no token_file or key_id")

        # 5. The iohr login.
        if used is None:
            program = cast("str", self.values.get("cli_path") or "iohr")
            if not self.allowed("cli"):
                skip("cli", "not in credential_sources")
            elif profile is None:
                skip("cli", "skipped, no profile chosen")
            elif table is None or "kind" not in table:
                skip("cli", f"profile {profile} was not made by iohr login")
            elif not self.found(program):
                skip(
                    "cli",
                    "iohr not found on PATH"
                    if program == "iohr"
                    else f"iohr not found at {program}",
                )
            else:
                tried.append({"source": "cli", "result": "used"})
                used = Credential("cli", "cli", cli_path=program, profile=profile)

        if used is None:
            lines = [f"no credentials found for profile {_q(profile or 'default')}; tried:"]
            lines += [f"  {t['source']}: {t.get('reason', '')}" for t in tried]
            lines.append(
                f"Set {p}KEY_ID, {p}KEY_SECRET and {p}SCOPES, or {p}TOKEN, or run `iohr login`."
            )
            self.problem("credential", "", "\n".join(lines))
            return None
        if used.kind not in ("client_credentials", "custom") and "scopes" in self.settings:
            s = self.settings.pop("scopes")
            self.values.pop("scopes", None)
            self.ignored.append(
                {"key": "scopes", "source": s["source"], "reason": "not used by this credential"}
            )
        self.tried = tried
        return used

    def cross_checks(self) -> None:
        get = self.settings.get
        trust = get("system_trust")
        if trust is not None and trust["value"] is False and get("ca_bundle") is None:
            self.problem(
                "system_trust",
                trust["source"],
                "system_trust = false needs a ca_bundle to trust instead",
            )
        cert, key = get("client_cert"), get("client_key")
        if cert is not None and key is None:
            self.problem("client_key", cert["source"], "client_cert needs client_key")
        if cert is None and key is not None:
            self.problem("client_cert", key["source"], "client_key needs client_cert")
        for name in ("ca_bundle", "client_cert", "client_key"):
            v = get(name)
            if v is not None and not self.exists(v["value"]):
                self.problem(name, v["source"], f"cannot read {v['value']}")


def resolve(
    code: Mapping[str, object],
    options: LoadOptions | None = None,
    *,
    profile_type: str | None = None,
    found: Callable[[str], bool] | None = None,
    read: Callable[[str], bytes | None] | None = None,
) -> Resolution:
    """Resolves a configuration the way `load` does (sections 2 to 5).

    Args:
        code: Options set in code, by catalogue name.
        options: The environment, OS, home and working directory to use.
        profile_type: Resolve for a typed (generated) profile of this name.
        found: Whether the `cli` source's program can run (default: a file, or on `PATH`).
        read: Reads a file, `None` when it cannot (default: the file system).

    Returns:
        The `describe()` document, the typed values and the chosen credential.

    Raises:
        ConfigError: Every problem found, in catalogue order.
    """
    o = options or LoadOptions()
    env: Mapping[str, str] = os.environ if o.env is None else o.env
    os_name = o.os or current_os()
    if o.home is None:
        home: str | None = os.path.expanduser("~")
        home = None if home == "~" else home
    else:
        home = o.home or None
    cwd = o.cwd or os.getcwd()
    r = _Resolver(
        code,
        env,
        os_name,
        home,
        cwd,
        profile_type,
        found or (lambda program: cli_found(program, env)),
        read or _default_read,
    )
    doc = r.load_file()
    chosen = r.choose_profile(doc)
    table: dict[str, Any] | None = None
    if chosen is not None:
        profiles = doc.get("profiles")
        if isinstance(profiles, dict):
            t = cast("dict[str, object]", profiles).get(chosen[0])
            table = cast("dict[str, Any]", t) if isinstance(t, dict) else None
    if table is not None and chosen is not None:
        r.layers.append((table, r.file_label(f"profiles.{chosen[0]}"), True))
    sdk = doc.get("sdk")
    if isinstance(sdk, dict):
        r.layers.append((cast("dict[str, Any]", sdk), r.file_label("sdk"), False))
    r.check_file_keys()
    r.resolve_settings()
    credential = r.chain(chosen[0] if chosen else None, table)
    r.cross_checks()
    if r.problems:
        problems = tuple(sorted(r.problems, key=lambda x: _order(x.setting)))
        raise ConfigError(format_problems(problems), problems)
    assert credential is not None
    r.values["profile"] = chosen[0] if chosen else None
    out: dict[str, Any] = {
        "profile": {"name": chosen[0], "source": chosen[1]} if chosen else None,
        "config_file": r.file_path,
        "settings": r.settings,
        "credential": {"source": credential.source, "kind": credential.kind, "tried": r.tried},
        "pipeline": list(PIPELINE),
        "ignored": r.ignored,
    }
    return Resolution(out, r.values, credential, r.file_path)


def format_problems(problems: Sequence[ConfigProblem]) -> str:
    """A `ConfigError`'s text: the chain's message alone, or every problem listed."""
    if len(problems) == 1 and problems[0].setting == "credential":
        return problems[0].message
    n = len(problems)
    lines = [f"configuration is invalid ({n} problem{'' if n == 1 else 's'}):"]
    for p in problems:
        first, *rest = p.message.splitlines() or [""]
        line = f"  {p.setting}: {first}"
        if p.source:
            line += f" (from {p.source})"
        lines.append(line)
        lines.extend(f"    {x}" for x in rest)
    return "\n".join(lines)
