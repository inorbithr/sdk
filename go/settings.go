package inorbit

import (
	"encoding/base64"
	"errors"
	"fmt"
	"net/url"
	"os"
	"regexp"
	"runtime"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/BurntSushi/toml"
)

// Configuration resolution (docs/config.md sections 2 to 5): each setting from code, the
// environment, the config file or its default, the credential chain, and the Describe
// document. Resolution reads files and the environment and contacts no host.

// builtIns is the built-in pipeline, outermost first (docs/config.md section 7.2).
var builtIns = []string{
	"request_id", "user_agent", "idempotency_key", "call_tracing", "deadline",
	"retry", "auth", "rate_limit", "attempt_tracing", "logging", "hooks", "timeout",
}

const (
	maxConfigFile = 1 << 20
	defaultCLI    = "iohr"
)

// cliKeys are the command line's own keys in a profile's table.
var cliKeys = []string{"kind", "account", "storage", "issuer", "client_id"}

// credentialSources are the sources the chain may use, in its order; code is always
// allowed.
var credentialSources = []string{"env", "workload", "file", "cli"}

// LoadOptions is what Load reads instead of the process: for tests, and for your own
// (docs/config.md section 9.2). The zero value reads the process.
type LoadOptions struct {
	// Env is the environment; nil reads the process's. An empty value is unset.
	Env map[string]string

	// OS is whose file locations apply: "linux", "macos" or "windows"; empty for this
	// machine's.
	OS string

	// Home is the home directory; empty for this machine's.
	Home string

	// NoHome resolves as if there were no home directory, as in a distroless container.
	NoHome bool

	// Cwd is the working directory relative paths in the environment and in code
	// resolve against; empty for the process's.
	Cwd string
}

type settingType int

const (
	tyDuration settingType = iota
	tyInt
	tyBool
	tyScopes
	tyList
	tyURL
	tyPath
	tySecret
	tyStr
	tyProxy
	tyPins
	tyReserved
	tyOneOf
)

type setting struct {
	name       string
	ty         settingType
	oneOf      []string
	file       bool // may appear in the config file
	credential bool // read only with a typed profile's prefix, chosen whole
	transport  bool // belongs to a caller-supplied HTTP client (section 6.6)
	fallback   any  // the default as Describe shows it; nil for none
}

func st(name string, ty settingType, fallback any) setting {
	return setting{name: name, ty: ty, file: true, fallback: fallback}
}

func credSetting(name string, ty settingType, file bool) setting {
	return setting{name: name, ty: ty, file: file, credential: true}
}

func netSetting(name string, ty settingType, fallback any) setting {
	s := st(name, ty, fallback)
	s.transport = true
	return s
}

func oneOf(name string, fallback any, values ...string) setting {
	s := st(name, tyOneOf, fallback)
	s.oneOf = values
	return s
}

// catalogue is section 3, in its order; problems are reported in this order.
var catalogue = []setting{
	st("base_url", tyURL, DefaultBaseURL),
	st("token_url", tyURL, DefaultTokenURL),
	st("region", tyReserved, nil),
	credSetting("key_id", tyStr, true),
	credSetting("key_secret", tySecret, false),
	credSetting("key_secret_file", tyPath, true),
	st("scopes", tyScopes, nil),
	credSetting("token", tySecret, false),
	credSetting("token_file", tyPath, true),
	st("credential_sources", tyList, nil),
	st("cli_path", tyPath, nil),
	netSetting("connect_timeout", tyDuration, "10s"),
	st("timeout", tyDuration, "30s"),
	st("total_timeout", tyDuration, "120s"),
	st("stream_idle_timeout", tyDuration, "45s"),
	st("max_retries", tyInt, 2),
	st("retry_base_delay", tyDuration, "500ms"),
	st("retry_max_delay", tyDuration, "8s"),
	st("retry_after_max", tyDuration, "60s"),
	st("retry_budget", tyBool, true),
	oneOf("streams", "sse", "sse", "socket"),
	netSetting("proxy", tyProxy, nil),
	netSetting("no_proxy", tyList, nil),
	netSetting("ca_bundle", tyPath, nil),
	netSetting("system_trust", tyBool, true),
	netSetting("client_cert", tyPath, nil),
	netSetting("client_key", tyPath, nil),
	{name: "client_key_password", ty: tySecret, transport: true},
	netSetting("pinned_keys", tyPins, nil),
	oneOf("log", "off", "off", "error", "warn", "info", "debug"),
	st("log_headers", tyBool, false),
	st("log_allow_headers", tyList, nil),
	st("tracing", tyBool, nil),
	st("metrics", tyBool, nil),
	oneOf("rate_limit", "observe", "observe", "wait", "off"),
	st("user_agent_suffix", tyStr, nil),
}

func settingNamed(name string) (setting, bool) {
	for _, s := range catalogue {
		if s.name == name {
			return s, true
		}
	}
	return setting{}, false
}

// order is a problem's place: profile and config_file first, the catalogue, then the
// credential.
func order(name string) int {
	switch name {
	case "profile":
		return 0
	case "config_file":
		return 1
	case "credential":
		return 1 << 30
	}
	for i, s := range catalogue {
		if s.name == name {
			return i + 2
		}
	}
	return 1<<30 - 1
}

var durationSyntax = regexp.MustCompile(`^(\d+)(ms|s|m|h)$`)

// ParseDuration reads the one duration syntax of the environment and the config file
// (docs/config.md section 2.4): digits, then ms, s, m or h, greater than zero. No bare
// numbers, fractions, spaces or compound values.
func ParseDuration(v string) (time.Duration, error) {
	m := durationSyntax.FindStringSubmatch(v)
	if m == nil {
		return 0, fmt.Errorf("%q is not a duration: digits, then ms, s, m or h, such as 30s", v)
	}
	n, err := strconv.ParseInt(m[1], 10, 64)
	unit := map[string]time.Duration{"ms": time.Millisecond, "s": time.Second, "m": time.Minute, "h": time.Hour}[m[2]]
	if err != nil || n <= 0 || n > int64(1<<62)/int64(unit) {
		return 0, fmt.Errorf("%q is not a duration greater than zero", v)
	}
	return time.Duration(n) * unit, nil
}

// showDuration is a duration as Describe shows it: 30s, or 1500ms.
func showDuration(d time.Duration) string {
	ms := (d + time.Millisecond - 1) / time.Millisecond
	if ms%1000 == 0 {
		return strconv.FormatInt(int64(ms/1000), 10) + "s"
	}
	return strconv.FormatInt(int64(ms), 10) + "ms"
}

// redactUserinfo hides a URL's user-info.
func redactUserinfo(raw string) string {
	i := strings.Index(raw, "://")
	if i < 0 {
		return raw
	}
	rest := raw[i+3:]
	end := strings.IndexAny(rest, "/?#")
	authority := rest
	if end >= 0 {
		authority = rest[:end]
	}
	at := strings.LastIndexByte(authority, '@')
	if at < 0 {
		return raw
	}
	return raw[:i] + "://" + redacted + "@" + rest[at+1:]
}

func hasUserinfo(raw string) bool {
	u, err := url.Parse(raw)
	return err == nil && u.User != nil
}

// pathRules are the path rules of one OS: absolute, join, parent.
type pathRules struct{ os string }

func (p pathRules) sep() string {
	if p.os == "windows" {
		return `\`
	}
	return "/"
}

var drive = regexp.MustCompile(`^[A-Za-z]:[\\/]`)

func (p pathRules) isAbs(path string) bool {
	if strings.HasPrefix(path, "/") {
		return true
	}
	if p.os == "windows" || drive.MatchString(path) {
		return strings.HasPrefix(path, `\`) || drive.MatchString(path)
	}
	return false
}

func trimSeparators(dir string) string {
	return strings.TrimRight(dir, `/\`)
}

func (p pathRules) join(dir, rest string) string {
	if p.os == "windows" {
		rest = strings.ReplaceAll(rest, "/", `\`)
	}
	return trimSeparators(dir) + p.sep() + rest
}

func (p pathRules) parent(path string) string {
	i := max(strings.LastIndexByte(path, '/'), strings.LastIndexByte(path, '\\'))
	var d string
	switch {
	case i == 0:
		d = path[:1]
	case i < 0:
		d = "."
	default:
		d = path[:i]
	}
	if p.os == "windows" {
		return strings.ReplaceAll(d, "/", `\`)
	}
	return strings.ReplaceAll(d, `\`, "/")
}

func currentOS() string {
	switch runtime.GOOS {
	case "darwin", "ios":
		return "macos"
	case "windows":
		return "windows"
	}
	return "linux"
}

// located is where the config file is: the path, whether it was named (code or
// INORBIT_CONFIG_FILE), and the source label.
type located struct {
	path  string
	named bool
	label string
}

// configPath finds the config file (docs/config.md section 4.1); nil reads no file.
func configPath(osName string, env func(string) string, home string, hasHome bool, code *string) *located {
	paths := pathRules{osName}
	if code != nil {
		if *code == "off" {
			return nil
		}
		return &located{path: *code, named: true, label: "code"}
	}
	if named := env("INORBIT_CONFIG_FILE"); named != "" {
		if named == "off" {
			return nil
		}
		return &located{path: named, named: true, label: "env INORBIT_CONFIG_FILE"}
	}
	if dir := env("IOHR_CONFIG_DIR"); dir != "" {
		return &located{path: paths.join(dir, "config.toml"), label: "env IOHR_CONFIG_DIR"}
	}
	found := func(p string) *located { return &located{path: p, label: "default"} }
	switch osName {
	case "macos":
		if !hasHome {
			return nil
		}
		return found(paths.join(home, "Library/Application Support/hr.InOrbit.iohr/config.toml"))
	case "windows":
		appdata := env("APPDATA")
		if appdata == "" {
			return nil
		}
		return found(paths.join(appdata, `InOrbit\iohr\config\config.toml`))
	}
	if xdg := env("XDG_CONFIG_HOME"); xdg != "" && paths.isAbs(xdg) {
		return found(paths.join(xdg, "iohr/config.toml"))
	}
	if !hasHome {
		return nil
	}
	return found(paths.join(home, ".config/iohr/config.toml"))
}

var profileName = regexp.MustCompile(`^[a-z0-9][a-z0-9_-]{0,63}$`)

// envName is a profile name in the environment's form: acme-ci is ACME_CI.
func envName(profile string) string {
	return strings.ReplaceAll(strings.ToUpper(profile), "-", "_")
}

func tomlType(v any) string {
	switch v.(type) {
	case string:
		return "string"
	case int64, int:
		return "integer"
	case float64:
		return "float"
	case bool:
		return "boolean"
	case time.Time:
		return "datetime"
	case []any, []map[string]any:
		return "array"
	}
	return "table"
}

func asTable(v any) (map[string]any, bool) {
	t, ok := v.(map[string]any)
	return t, ok
}

// rawFrom is where a raw value came from.
type rawFrom int

const (
	fromCode rawFrom = iota
	fromEnv
	fromFile
)

type rawValue struct {
	from  rawFrom
	value any
}

type layer struct {
	table   map[string]any
	label   string
	profile bool // the profile's own table, where the command line's keys are not SDK keys
}

// DescribedSetting is one setting as Describe shows it.
type DescribedSetting struct {
	// Value is the value: a string (durations as 30s, URLs with user-info redacted,
	// secrets as <redacted>), a number, a boolean or a list of strings.
	Value any `json:"value"`

	// Source is where it came from: code, env INORBIT_TIMEOUT, file <path> [sdk], default.
	Source string `json:"source"`
}

// DescribedProfile is the profile the configuration was resolved for.
type DescribedProfile struct {
	// Name is the profile's name.
	Name string `json:"name"`

	// Source is what chose it: code, env INORBIT_PROFILE, or file <path> for the
	// file's default.
	Source string `json:"source"`
}

// TriedSource is one source of the credential chain, as Describe lists it.
type TriedSource struct {
	// Source is code, env, workload, file or cli.
	Source string `json:"source"`

	// Result is used or skipped.
	Result string `json:"result"`

	// Reason says why a source was skipped.
	Reason string `json:"reason,omitempty"`
}

// DescribedCredential is the credential the chain chose.
type DescribedCredential struct {
	// Source is the source used: code, env, file or cli.
	Source string `json:"source"`

	// Kind is custom, static_token, token_file, client_credentials or cli.
	Kind string `json:"kind"`

	// Tried lists every source tried, in order.
	Tried []TriedSource `json:"tried"`
}

// IgnoredSetting is a value that was read and not used.
type IgnoredSetting struct {
	// Key is the key or setting.
	Key string `json:"key"`

	// Source is where it was read.
	Source string `json:"source"`

	// Reason says why it is not used.
	Reason string `json:"reason"`
}

// Description is the Describe document (docs/config.md section 2.6), stable within a
// major version; encoding/json writes it in the documented form.
type Description struct {
	// Profile is the profile resolved for; nil for none.
	Profile *DescribedProfile `json:"profile"`

	// ConfigFile is the config file read; nil for none.
	ConfigFile *string `json:"config_file"`

	// Settings holds every setting with a value, defaults included.
	Settings map[string]DescribedSetting `json:"settings"`

	// Credential is the credential chosen.
	Credential *DescribedCredential `json:"credential"`

	// Pipeline is the final pipeline, by name, outermost first.
	Pipeline []string `json:"pipeline"`

	// Ignored lists what was read and not used.
	Ignored []IgnoredSetting `json:"ignored"`
}

// credentialPlan is what the chain decided: the credential's shape and its pieces.
type credentialPlan struct {
	source        string
	kind          string // custom, static_token, token_file, client_credentials, cli
	token         string
	path          string
	keyID         string
	keySecret     string
	keySecretFile string
	profile       string
	program       string
}

// resolution is a resolved configuration: the values a client is built from, and its
// description.
type resolution struct {
	values      map[string]any
	description Description
	credential  credentialPlan
	profile     string
	proxy       *proxyChoice
}

// resolveInput is what resolve is handed.
type resolveInput struct {
	code          map[string]any
	httpClient    bool
	tokenProvider bool
	profileType   string
	explicit      bool
	load          LoadOptions
}

type fileRead struct {
	data    []byte
	tooBig  bool
	missing bool
}

func readSmallFile(path string) fileRead {
	info, err := os.Stat(path)
	if err != nil || !info.Mode().IsRegular() {
		return fileRead{missing: true}
	}
	if info.Size() > maxConfigFile {
		return fileRead{tooBig: true}
	}
	data, err := os.ReadFile(path) //nolint:gosec // the configured file, by design
	if err != nil {
		return fileRead{missing: true}
	}
	return fileRead{data: data}
}

func readable(path string) bool {
	f, err := os.Open(path) //nolint:gosec // a configured file, by design
	if err != nil {
		return false
	}
	_ = f.Close()
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

// programFound reports whether program can be run: a path to a file, or a name found
// on PATH. PATH is read with this machine's rules (separator, Windows extensions),
// whatever OS the resolution follows.
func programFound(program string, env func(string) string) bool {
	isFile := func(p string) bool {
		info, err := os.Stat(p)
		return err == nil && info.Mode().IsRegular()
	}
	if strings.ContainsAny(program, `/\`) {
		return isFile(program)
	}
	path := env("PATH")
	if path == "" {
		path = env("Path")
	}
	if path == "" {
		return false
	}
	windows := runtime.GOOS == "windows"
	exts := []string{""}
	sep := "/"
	if windows {
		exts = []string{"", ".exe", ".cmd", ".bat"}
		sep = `\`
	}
	for _, dir := range strings.Split(path, string(os.PathListSeparator)) {
		if dir == "" {
			continue
		}
		for _, ext := range exts {
			if isFile(trimSeparators(dir) + sep + program + ext) {
				return true
			}
		}
	}
	return false
}

// resolve resolves a configuration, or returns a ConfigError listing every problem.
func resolve(in resolveInput) (*resolution, error) {
	r := &resolver{in: in, settings: map[string]DescribedSetting{}, values: map[string]any{}}
	r.init()
	return r.run()
}

type resolver struct {
	in       resolveInput
	envMap   map[string]string
	useProc  bool
	os       string
	paths    pathRules
	home     string
	hasHome  bool
	cwd      string
	prefix   string // INORBIT_<P>_ for a typed profile
	problems []ConfigProblem
	settings map[string]DescribedSetting
	values   map[string]any
	ignored  []IgnoredSetting
	layers   []layer
	filePath string
	hasFile  bool
	fileDir  string
	proxy    *proxyChoice
}

func (r *resolver) init() {
	lo := r.in.load
	switch {
	case r.in.explicit:
		r.envMap = map[string]string{}
	case lo.Env != nil:
		r.envMap = lo.Env
	default:
		r.useProc = true
	}
	r.os = lo.OS
	if r.os == "" {
		r.os = currentOS()
	}
	r.paths = pathRules{r.os}
	switch {
	case lo.NoHome:
	case lo.Home != "":
		r.home, r.hasHome = lo.Home, true
	default:
		if h, err := os.UserHomeDir(); err == nil && h != "" {
			r.home, r.hasHome = h, true
		}
	}
	r.cwd = lo.Cwd
	if r.cwd == "" {
		if wd, err := os.Getwd(); err == nil {
			r.cwd = wd
		} else {
			r.cwd = "."
		}
	}
	if r.in.profileType != "" {
		r.prefix = "INORBIT_" + envName(r.in.profileType) + "_"
	}
}

// v is an environment variable; empty is unset.
func (r *resolver) v(k string) string {
	if r.useProc {
		return os.Getenv(k)
	}
	return r.envMap[k]
}

func (r *resolver) problem(setting, source, message string) {
	r.problems = append(r.problems, ConfigProblem{Setting: setting, Source: source, Message: message})
}

func (r *resolver) label(suffix string) string {
	if !r.hasFile {
		return ""
	}
	if suffix == "" {
		return "file " + r.filePath
	}
	return "file " + r.filePath + " [" + suffix + "]"
}

func (r *resolver) run() (*resolution, error) {
	doc := map[string]any{}
	if !r.in.explicit {
		doc = r.readFile()
	}
	var profile *DescribedProfile
	switch {
	case r.in.profileType != "":
		profile = &DescribedProfile{Name: r.in.profileType, Source: "code"}
	case !r.in.explicit:
		profile = r.chooseProfile(doc)
	}
	profiles, _ := asTable(doc["profiles"])
	var table map[string]any
	if profile != nil {
		if t, ok := asTable(profiles[profile.Name]); ok {
			table = t
			r.layers = append(r.layers, layer{table: t, label: r.label("profiles." + profile.Name), profile: true})
		}
	}
	if sdk, ok := asTable(doc["sdk"]); ok {
		r.layers = append(r.layers, layer{table: sdk, label: r.label("sdk")})
	}
	if r.hasFile {
		r.fileDir = r.paths.parent(r.filePath)
	} else {
		r.fileDir = r.cwd
	}
	r.checkFileKeys()
	r.resolveSettings()
	var plan *credentialPlan
	var described *DescribedCredential
	if r.in.explicit {
		plan, described = r.explicitCredential()
	} else {
		name := ""
		if profile != nil {
			name = profile.Name
		}
		plan, described = r.chain(name, profile != nil, table)
	}
	r.crossChecks()
	if len(r.problems) > 0 {
		problems := slices.Clone(r.problems)
		sort.SliceStable(problems, func(i, j int) bool { return order(problems[i].Setting) < order(problems[j].Setting) })
		return nil, configErrorOf(problems)
	}
	if plan == nil {
		return nil, &ConfigError{Message: "no credentials"}
	}
	d := Description{
		Profile: profile, Settings: r.settings, Credential: described,
		Pipeline: slices.Clone(builtIns), Ignored: r.ignored,
	}
	if d.Ignored == nil {
		d.Ignored = []IgnoredSetting{}
	}
	if r.hasFile {
		d.ConfigFile = Ptr(r.filePath)
	}
	res := &resolution{values: r.values, description: d, credential: *plan, proxy: r.proxy}
	if profile != nil {
		res.profile = profile.Name
	}
	return res, nil
}

func (r *resolver) readFile() map[string]any {
	var code *string
	if v, ok := r.in.code["config_file"].(string); ok {
		code = &v
	}
	loc := configPath(r.os, r.v, r.home, r.hasHome, code)
	if loc == nil {
		return map[string]any{}
	}
	abs := loc.path
	if !r.paths.isAbs(abs) {
		abs = r.paths.join(r.cwd, abs)
	}
	read := readSmallFile(abs)
	switch {
	case read.missing:
		if loc.named {
			r.problem("config_file", loc.label, "there is no readable file at "+abs)
		}
		return map[string]any{}
	case read.tooBig:
		r.problem("config_file", loc.label, abs+" is larger than 1 MiB")
		return map[string]any{}
	case !utf8.Valid(read.data):
		r.problem("config_file", loc.label, abs+" is not valid TOML: it is not UTF-8")
		return map[string]any{}
	}
	doc := map[string]any{}
	if _, err := toml.Decode(string(read.data), &doc); err != nil {
		// The first line only: the parser's excerpt could quote a secret.
		msg := err.Error()
		var perr toml.ParseError
		if errors.As(err, &perr) {
			msg = fmt.Sprintf("line %d: %s", perr.Position.Line, perr.Message)
		}
		first, _, _ := strings.Cut(msg, "\n")
		r.problem("config_file", loc.label, abs+" is not valid TOML: "+first)
		return map[string]any{}
	}
	r.filePath, r.hasFile = abs, true
	return doc
}

func (r *resolver) chooseProfile(doc map[string]any) *DescribedProfile {
	var chosen *DescribedProfile
	if v, ok := r.in.code["profile"].(string); ok && v != "" {
		chosen = &DescribedProfile{Name: v, Source: "code"}
	} else if v := r.v("INORBIT_PROFILE"); v != "" {
		chosen = &DescribedProfile{Name: v, Source: "env INORBIT_PROFILE"}
	} else if v, ok := doc["default"].(string); ok {
		chosen = &DescribedProfile{Name: v, Source: r.label("")}
	}
	if chosen == nil {
		return nil
	}
	profiles, _ := asTable(doc["profiles"])
	if !profileName.MatchString(chosen.Name) {
		r.problem("profile", chosen.Source, fmt.Sprintf("%q is not a profile name: 1 to 64 lower-case letters, digits, '-' or '_', starting with a letter or digit", chosen.Name))
		return nil
	}
	if _, ok := profiles[chosen.Name]; !ok {
		where := "no config file was read"
		if r.hasFile {
			where = fmt.Sprintf("%s has no [profiles.%s]", r.filePath, chosen.Name)
		}
		r.problem("profile", chosen.Source, fmt.Sprintf("there is no profile %q: %s; `iohr profile list` shows the profiles", chosen.Name, where))
		return nil
	}
	return chosen
}

func (r *resolver) checkFileKeys() {
	for _, l := range r.layers {
		keys := make([]string, 0, len(l.table))
		for k := range l.table {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			v := l.table[k]
			if l.profile && slices.Contains(cliKeys, k) {
				continue
			}
			s, known := settingNamed(k)
			switch {
			case !known:
				reason := "unknown key"
				if k == "profile" || k == "config_file" {
					reason = "not read from the config file"
				}
				r.ignored = append(r.ignored, IgnoredSetting{Key: k, Source: l.label, Reason: reason})
			case !s.file:
				way := "the environment or code"
				switch k {
				case "key_secret":
					way = "key_secret_file, the environment, or iohr login"
				case "token":
					way = "token_file, the environment, or iohr login"
				}
				r.problem(k, l.label, "secrets are not allowed in the config file; use "+way)
			case s.ty == tyProxy:
				if str, ok := v.(string); ok && hasUserinfo(str) {
					r.problem(k, l.label, "a proxy URL with a user name or password holds a secret, which is not allowed in the config file; set it in INORBIT_PROXY or in code")
				}
			}
		}
	}
}

func (r *resolver) envNames(s setting) []string {
	upper := strings.ToUpper(s.name)
	var names []string
	if r.prefix != "" {
		names = append(names, r.prefix+upper)
	}
	if !s.credential || r.prefix == "" {
		names = append(names, "INORBIT_"+upper)
	}
	return names
}

func (r *resolver) raw(s setting) (rawValue, string, bool) {
	if v, ok := r.in.code[s.name]; ok {
		return rawValue{fromCode, v}, "code", true
	}
	for _, n := range r.envNames(s) {
		if v := r.v(n); v != "" {
			return rawValue{fromEnv, v}, "env " + n, true
		}
	}
	if s.file {
		for _, l := range r.layers {
			if v, ok := l.table[s.name]; ok {
				return rawValue{fromFile, v}, l.label, true
			}
		}
	}
	var after []string
	switch s.name {
	case "proxy":
		after = []string{"https_proxy", "HTTPS_PROXY"}
	case "no_proxy":
		after = []string{"no_proxy", "NO_PROXY"}
	}
	for _, n := range after {
		if v := r.v(n); v != "" {
			return rawValue{fromEnv, v}, "env " + n, true
		}
	}
	return rawValue{}, "", false
}

func (r *resolver) set(name string, shown, value any, source string) {
	r.settings[name] = DescribedSetting{Value: shown, Source: source}
	r.values[name] = value
}

func defaultValue(s setting) any {
	if s.ty == tyDuration {
		if str, ok := s.fallback.(string); ok {
			d, _ := ParseDuration(str)
			return d
		}
	}
	return s.fallback
}

func (r *resolver) resolveSettings() {
	for _, s := range catalogue {
		if s.credential {
			continue
		}
		raw, source, found := r.raw(s)
		if !found {
			if s.fallback != nil && (!s.transport || !r.in.httpClient) {
				r.set(s.name, s.fallback, defaultValue(s), "default")
			}
			continue
		}
		if s.transport && r.in.httpClient {
			if source == "code" {
				r.problem(s.name, "code", "configure this on your HTTP client (WithHTTPClient or WithTransport), or leave http_client out")
			} else {
				r.ignored = append(r.ignored, IgnoredSetting{Key: s.name, Source: source, Reason: "the caller's HTTP client decides this"})
			}
			continue
		}
		shown, value, err := r.parse(s, raw)
		if err != nil {
			r.problem(s.name, source, err.Error())
			continue
		}
		r.set(s.name, shown, value, source)
		if s.name == "proxy" {
			r.proxy = &proxyChoice{url: value.(string), explicit: !strings.HasPrefix(source, "env ") || source == "env INORBIT_PROXY"}
		}
	}
}

func (r *resolver) path(p string, fromFile bool) (string, error) {
	if strings.HasPrefix(p, "~/") {
		if !r.hasHome {
			return "", fmt.Errorf("%s starts with ~/ but there is no home directory", p)
		}
		return r.paths.join(r.home, p[2:]), nil
	}
	if r.paths.isAbs(p) {
		return p, nil
	}
	if fromFile {
		return r.paths.join(r.fileDir, p), nil
	}
	return r.paths.join(r.cwd, p), nil
}

func splitList(v string, comma bool) []string {
	if !comma {
		return strings.Fields(v)
	}
	var out []string
	for _, x := range strings.Split(v, ",") {
		if x = strings.TrimSpace(x); x != "" {
			out = append(out, x)
		}
	}
	return out
}

// parse is a raw value as Describe shows it and as the client uses it.
func (r *resolver) parse(s setting, raw rawValue) (any, any, error) {
	text := func(what string) (string, error) {
		if str, ok := raw.value.(string); ok {
			return str, nil
		}
		if raw.from == fromFile {
			return "", fmt.Errorf("must be %s, not %s", what, tomlType(raw.value))
		}
		return "", fmt.Errorf("must be %s", what)
	}
	list := func(comma bool) ([]string, error) {
		switch v := raw.value.(type) {
		case string:
			return splitList(v, comma), nil
		case []string:
			return slices.Clone(v), nil
		case []any:
			out := make([]string, 0, len(v))
			for _, x := range v {
				str, ok := x.(string)
				if !ok {
					if raw.from == fromFile {
						return nil, errors.New("must be an array of strings")
					}
					return nil, errors.New("must be a list of strings")
				}
				out = append(out, str)
			}
			return out, nil
		}
		if raw.from == fromFile {
			return nil, fmt.Errorf("must be an array of strings, not %s", tomlType(raw.value))
		}
		return nil, errors.New("must be a list of strings")
	}
	switch s.ty {
	case tyOneOf:
		v, err := text("a string")
		if err != nil {
			return nil, nil, err
		}
		if !slices.Contains(s.oneOf, v) {
			return nil, nil, fmt.Errorf("%q is not one of %s", v, strings.Join(s.oneOf, ", "))
		}
		return v, v, nil
	case tyDuration:
		if raw.from == fromCode {
			d, ok := raw.value.(time.Duration)
			if !ok || d <= 0 {
				return nil, nil, errors.New("must be a duration greater than zero")
			}
			return showDuration(d), d, nil
		}
		v, err := text(`a duration string such as "30s"`)
		if err != nil {
			return nil, nil, err
		}
		d, perr := ParseDuration(v)
		if perr != nil {
			if _, e := ParseDuration(v + "s"); e == nil {
				return nil, nil, fmt.Errorf("%q is not a duration; write it with a unit, such as 30s", v)
			}
			return nil, nil, fmt.Errorf("%q is not a duration greater than zero: digits, then ms, s, m or h, such as 30s", v)
		}
		return showDuration(d), d, nil
	case tyInt:
		switch v := raw.value.(type) {
		case string:
			n, ok := digits(v)
			if raw.from != fromEnv || !ok || n > 0xffffffff {
				if raw.from == fromFile {
					return nil, nil, fmt.Errorf("must be an integer, not %s", tomlType(v))
				}
				return nil, nil, fmt.Errorf("%q is not a whole number of 0 or more", v)
			}
			return int(n), int(n), nil
		case int64:
			if v >= 0 && v <= 0xffffffff {
				return int(v), int(v), nil
			}
		case int:
			if v >= 0 && v <= 0xffffffff {
				return v, v, nil
			}
		default:
			if raw.from == fromFile {
				return nil, nil, fmt.Errorf("must be an integer, not %s", tomlType(v))
			}
		}
		return nil, nil, errors.New("must be a whole number of 0 or more")
	case tyBool:
		switch v := raw.value.(type) {
		case bool:
			return v, v, nil
		case string:
			if raw.from == fromEnv {
				switch strings.ToLower(v) {
				case "true", "1":
					return true, true, nil
				case "false", "0":
					return false, false, nil
				}
				return nil, nil, fmt.Errorf("%q is not true, false, 1 or 0", v)
			}
		}
		if raw.from == fromFile {
			return nil, nil, fmt.Errorf("must be a boolean, not %s", tomlType(raw.value))
		}
		return nil, nil, errors.New("must be a boolean")
	case tyScopes:
		l, err := list(false)
		if err != nil {
			return nil, nil, err
		}
		return l, l, nil
	case tyList:
		l, err := list(true)
		if err != nil {
			return nil, nil, err
		}
		switch s.name {
		case "credential_sources":
			for _, x := range l {
				if !slices.Contains(credentialSources, x) {
					return nil, nil, fmt.Errorf("%q is not a credential source; use env, workload, file or cli", x)
				}
			}
		case "no_proxy":
			if _, bad, ok := parseNoProxy(l); !ok {
				return nil, nil, fmt.Errorf("%q is not a no_proxy entry: a host, .domain, host:port, an IP address or a CIDR range", bad)
			}
		case "log_allow_headers":
			for i := range l {
				l[i] = strings.ToLower(l[i])
			}
		}
		if l == nil {
			l = []string{}
		}
		return l, l, nil
	case tyURL:
		v, err := text("a URL string")
		if err != nil {
			return nil, nil, err
		}
		if err := checkSettingURL(v, s.name == "base_url"); err != nil {
			return nil, nil, err
		}
		return v, v, nil
	case tyPath:
		v, err := text("a path string")
		if err != nil {
			return nil, nil, err
		}
		p, err := r.path(v, raw.from == fromFile)
		if err != nil {
			return nil, nil, err
		}
		return p, p, nil
	case tySecret:
		if str, ok := raw.value.(string); ok {
			return redacted, str, nil
		}
		return nil, nil, errors.New("must be a string")
	case tyStr:
		v, err := text("a string")
		if err != nil {
			return nil, nil, err
		}
		if s.name == "user_agent_suffix" && (utf8.RuneCountInString(v) > 128 || !printableASCII(v)) {
			return nil, nil, errors.New("must be product tokens (such as myapp/1.2), at most 128 printable ASCII characters")
		}
		return v, v, nil
	case tyProxy:
		v, err := text("a URL string")
		if err != nil {
			return nil, nil, err
		}
		if v == "off" {
			return "off", "off", nil
		}
		shown := redactUserinfo(v)
		u, perr := url.Parse(v)
		if perr != nil || u.Host == "" {
			return nil, nil, fmt.Errorf("%q is not an absolute URL", shown)
		}
		if u.Scheme != "http" && u.Scheme != "https" {
			return nil, nil, fmt.Errorf("%q must be an http:// or https:// proxy URL, or off", shown)
		}
		return shown, v, nil
	case tyPins:
		l, err := list(true)
		if err != nil {
			return nil, nil, err
		}
		if len(l) < 2 {
			return nil, nil, errors.New("pin at least two keys (the current one and a backup)")
		}
		for _, p := range l {
			b, err := base64.StdEncoding.DecodeString(p)
			if err != nil || len(b) != 32 {
				return nil, nil, fmt.Errorf("%q is not a base64 SHA-256 of a public key", p)
			}
		}
		return l, l, nil
	}
	return nil, nil, errors.New("region is reserved until the API offers regions; remove it")
}

func printableASCII(s string) bool {
	for i := range len(s) {
		if s[i] < 0x20 || s[i] > 0x7e {
			return false
		}
	}
	return true
}

// checkSettingURL holds a URL to SR-07: https, or http to loopback only, no
// credentials or fragment; the base URL is an origin only.
func checkSettingURL(v string, originOnly bool) error {
	u, err := url.Parse(v)
	if err != nil || u.Scheme == "" || u.Host == "" {
		return fmt.Errorf("%q is not an absolute URL", v)
	}
	if u.Scheme != "https" && (u.Scheme != "http" || !isLoopback(u.Hostname())) {
		return fmt.Errorf("%q must use https (plain http is allowed only for localhost and loopback addresses)", v)
	}
	if u.User != nil || u.Fragment != "" || strings.HasSuffix(v, "#") {
		return fmt.Errorf("%q must not carry credentials or a fragment", v)
	}
	if originOnly && (strings.TrimSuffix(u.Path, "/") != "" || u.RawQuery != "" || u.ForceQuery) {
		return fmt.Errorf("%q is an origin only, such as https://api.inorbit.hr", v)
	}
	return nil
}

func (r *resolver) allowed(source string) bool {
	l, ok := r.values["credential_sources"].([]string)
	return !ok || slices.Contains(l, source)
}

func (r *resolver) show(name string, value any, source string) {
	r.settings[name] = DescribedSetting{Value: value, Source: source}
}

func (r *resolver) codeString(k string) string {
	v, _ := r.in.code[k].(string)
	return v
}

func (r *resolver) explicitCredential() (*credentialPlan, *DescribedCredential) {
	var plan credentialPlan
	switch {
	case r.in.tokenProvider:
		plan = credentialPlan{source: "code", kind: "custom"}
	case r.codeString("token") != "":
		plan = credentialPlan{source: "code", kind: "static_token", token: r.codeString("token")}
		r.show("token", redacted, "code")
	case r.codeString("token_file") != "":
		plan = credentialPlan{source: "code", kind: "token_file", path: r.codeString("token_file")}
		r.show("token_file", r.codeString("token_file"), "code")
	default:
		keyID := r.codeString("key_id")
		r.show("key_id", keyID, "code")
		if f := r.codeString("key_secret_file"); f != "" && r.codeString("key_secret") == "" {
			plan = credentialPlan{source: "code", kind: "client_credentials", keyID: keyID, keySecretFile: f}
			r.show("key_secret_file", f, "code")
		} else {
			plan = credentialPlan{source: "code", kind: "client_credentials", keyID: keyID, keySecret: r.codeString("key_secret")}
			r.show("key_secret", redacted, "code")
		}
	}
	r.scopesFor(plan.kind)
	return &plan, &DescribedCredential{Source: "code", Kind: plan.kind, Tried: []TriedSource{{Source: "code", Result: "used"}}}
}

// chain is section 5.1; nil when a problem stopped it.
func (r *resolver) chain(profile string, hasProfile bool, table map[string]any) (*credentialPlan, *DescribedCredential) {
	var tried []TriedSource
	skip := func(source, reason string) {
		tried = append(tried, TriedSource{Source: source, Result: "skipped", Reason: reason})
	}
	p := r.prefix
	if p == "" {
		p = "INORBIT_"
	}
	var plan *credentialPlan

	// 1. Code.
	switch {
	case r.in.tokenProvider:
		plan = &credentialPlan{source: "code", kind: "custom"}
	case r.codeString("token") != "":
		plan = &credentialPlan{source: "code", kind: "static_token", token: r.codeString("token")}
		r.show("token", redacted, "code")
	case r.codeString("token_file") != "":
		path, err := r.path(r.codeString("token_file"), false)
		if err != nil || !readable(path) {
			r.problem("token_file", "code", "cannot read "+path)
			return nil, nil
		}
		plan = &credentialPlan{source: "code", kind: "token_file", path: path}
		r.show("token_file", path, "code")
	case r.codeString("key_id") != "":
		keyID, secret, secretFile := r.codeString("key_id"), r.codeString("key_secret"), r.codeString("key_secret_file")
		switch {
		case secret != "" && secretFile != "":
			r.problem("key_secret", "code", "WithKey and WithKeyFile are both set; set one")
			return nil, nil
		case secret == "" && secretFile == "":
			r.problem("key_secret", "code", "a key id is set without a secret or a secret file")
			return nil, nil
		}
		if _, ok := r.values["scopes"]; !ok {
			r.problem("scopes", "code", "a key needs scopes: set WithScopes")
			return nil, nil
		}
		r.show("key_id", keyID, "code")
		if secret != "" {
			plan = &credentialPlan{source: "code", kind: "client_credentials", keyID: keyID, keySecret: secret}
			r.show("key_secret", redacted, "code")
		} else {
			path, err := r.path(secretFile, false)
			if err != nil || !readable(path) {
				r.problem("key_secret_file", "code", "cannot read "+path)
				return nil, nil
			}
			plan = &credentialPlan{source: "code", kind: "client_credentials", keyID: keyID, keySecretFile: path}
			r.show("key_secret_file", path, "code")
		}
	}
	if plan != nil {
		tried = append(tried, TriedSource{Source: "code", Result: "used"})
	} else {
		skip("code", "none set")
	}

	// 2. The environment.
	if plan == nil {
		n := func(x string) string { return p + x }
		src := func(x string) string { return "env " + p + x }
		token, tokenFile, keyID := r.v(n("TOKEN")), r.v(n("TOKEN_FILE")), r.v(n("KEY_ID"))
		secret, secretFile := r.v(n("KEY_SECRET")), r.v(n("KEY_SECRET_FILE"))
		switch {
		case !r.allowed("env"):
			skip("env", "not in credential_sources")
		case token == "" && tokenFile == "" && keyID == "":
			skip("env", fmt.Sprintf("%s, %s and %s are not set", n("TOKEN"), n("TOKEN_FILE"), n("KEY_ID")))
		default:
			var set []string
			for _, kv := range [][2]string{{"TOKEN", token}, {"TOKEN_FILE", tokenFile}, {"KEY_ID", keyID}} {
				if kv[1] != "" {
					set = append(set, kv[0])
				}
			}
			if len(set) > 1 {
				names := make([]string, len(set))
				for i, s := range set {
					names[i] = n(s)
				}
				r.problem(strings.ToLower(set[0]), src(set[0]), strings.Join(names, " and ")+" are both set; set one credential")
				return nil, nil
			}
			switch {
			case token != "":
				r.show("token", redacted, src("TOKEN"))
				plan = &credentialPlan{source: "env", kind: "static_token", token: token}
			case tokenFile != "":
				path, err := r.path(tokenFile, false)
				if err != nil || !readable(path) {
					r.problem("token_file", src("TOKEN_FILE"), "cannot read "+path)
					return nil, nil
				}
				r.show("token_file", path, src("TOKEN_FILE"))
				plan = &credentialPlan{source: "env", kind: "token_file", path: path}
			default:
				switch {
				case secret != "" && secretFile != "":
					r.problem("key_secret", src("KEY_SECRET"), fmt.Sprintf("%s and %s are both set; set one", n("KEY_SECRET"), n("KEY_SECRET_FILE")))
					return nil, nil
				case secret == "" && secretFile == "":
					r.problem("key_secret", src("KEY_ID"), fmt.Sprintf("%s is set without %s or %s", n("KEY_ID"), n("KEY_SECRET"), n("KEY_SECRET_FILE")))
					return nil, nil
				}
				if _, ok := r.values["scopes"]; !ok {
					r.problem("scopes", src("KEY_ID"), "a key needs scopes: set "+p+"SCOPES")
					return nil, nil
				}
				r.show("key_id", keyID, src("KEY_ID"))
				if secret != "" {
					r.show("key_secret", redacted, src("KEY_SECRET"))
					plan = &credentialPlan{source: "env", kind: "client_credentials", keyID: keyID, keySecret: secret}
				} else {
					path, err := r.path(secretFile, false)
					if err != nil || !readable(path) {
						r.problem("key_secret_file", src("KEY_SECRET_FILE"), "cannot read "+path)
						return nil, nil
					}
					r.show("key_secret_file", path, src("KEY_SECRET_FILE"))
					plan = &credentialPlan{source: "env", kind: "client_credentials", keyID: keyID, keySecretFile: path}
				}
			}
			tried = append(tried, TriedSource{Source: "env", Result: "used"})
		}
	}

	// 3. Workload identity: reserved until the platform exchanges outside tokens.
	if plan == nil {
		if r.allowed("workload") {
			skip("workload", "not offered by the platform yet")
		} else {
			skip("workload", "not in credential_sources")
		}
	}

	// 4. The config file's profile table.
	if plan == nil {
		label := ""
		if len(r.layers) > 0 {
			label = r.layers[0].label
		}
		_, hasTokenFile := table["token_file"]
		_, hasKeyID := table["key_id"]
		switch {
		case !r.allowed("file"):
			skip("file", "not in credential_sources")
		case !r.hasFile:
			skip("file", "no config file was read")
		case !hasProfile:
			skip("file", "no profile chosen")
		case table != nil && (hasTokenFile || hasKeyID):
			if hasTokenFile && hasKeyID {
				r.problem("token_file", label, "token_file and key_id are both set; set one credential")
				return nil, nil
			}
			if hasTokenFile {
				tf, ok := table["token_file"].(string)
				if !ok {
					r.problem("token_file", label, "must be a path string")
					return nil, nil
				}
				path := r.safePath(tf)
				if !readable(path) {
					r.problem("token_file", label, "cannot read "+path)
					return nil, nil
				}
				r.show("token_file", path, label)
				plan = &credentialPlan{source: "file", kind: "token_file", path: path}
			} else {
				keyID, ok := table["key_id"].(string)
				if !ok {
					r.problem("key_id", label, "must be a string")
					return nil, nil
				}
				if _, has := table["key_secret"]; has {
					return nil, nil // Already reported: secrets are not allowed in the file.
				}
				f, ok := table["key_secret_file"].(string)
				if !ok {
					r.problem("key_secret", label, "key_id is set without key_secret_file")
					return nil, nil
				}
				if _, ok := r.values["scopes"]; !ok {
					r.problem("scopes", label, "a key needs scopes: set scopes in the profile's table")
					return nil, nil
				}
				path := r.safePath(f)
				if !readable(path) {
					r.problem("key_secret_file", label, "cannot read "+path)
					return nil, nil
				}
				r.show("key_id", keyID, label)
				r.show("key_secret_file", path, label)
				plan = &credentialPlan{source: "file", kind: "client_credentials", keyID: keyID, keySecretFile: path}
			}
			tried = append(tried, TriedSource{Source: "file", Result: "used"})
		default:
			skip("file", "profile "+profile+" sets no token_file or key_id")
		}
	}

	// 5. The iohr login.
	if plan == nil {
		program := defaultCLI
		if v, ok := r.values["cli_path"].(string); ok && v != "" {
			program = v
		}
		_, made := table["kind"]
		switch {
		case !r.allowed("cli"):
			skip("cli", "not in credential_sources")
		case !hasProfile:
			skip("cli", "skipped, no profile chosen")
		case table == nil || !made:
			skip("cli", "profile "+profile+" was not made by iohr login")
		case !programFound(program, r.v):
			if program == defaultCLI {
				skip("cli", "iohr not found on PATH")
			} else {
				skip("cli", "iohr not found at "+program)
			}
		default:
			tried = append(tried, TriedSource{Source: "cli", Result: "used"})
			plan = &credentialPlan{source: "cli", kind: "cli", profile: profile, program: program}
		}
	}

	if plan == nil {
		var b strings.Builder
		name := profile
		if name == "" {
			name = "default"
		}
		fmt.Fprintf(&b, "no credentials found for profile %q; tried:", name)
		for _, t := range tried {
			fmt.Fprintf(&b, "\n  %s: %s", t.Source, t.Reason)
		}
		fmt.Fprintf(&b, "\nSet %sKEY_ID, %sKEY_SECRET and %sSCOPES, or %sTOKEN, or run `iohr login`.", p, p, p, p)
		r.problem("credential", "", b.String())
		return nil, nil
	}
	r.scopesFor(plan.kind)
	return plan, &DescribedCredential{Source: plan.source, Kind: plan.kind, Tried: tried}
}

func (r *resolver) safePath(p string) string {
	out, err := r.path(p, true)
	if err != nil {
		return p
	}
	return out
}

// scopesFor lists scopes next to a credential that carries its own as ignored.
func (r *resolver) scopesFor(kind string) {
	if kind == "client_credentials" || kind == "custom" {
		return
	}
	if s, ok := r.settings["scopes"]; ok {
		delete(r.settings, "scopes")
		delete(r.values, "scopes")
		r.ignored = append(r.ignored, IgnoredSetting{Key: "scopes", Source: s.Source, Reason: "not used by this credential"})
	}
}

func (r *resolver) crossChecks() {
	trust, hasTrust := r.settings["system_trust"]
	if _, hasBundle := r.settings["ca_bundle"]; hasTrust && trust.Value == false && !hasBundle {
		r.problem("system_trust", trust.Source, "system_trust = false needs a ca_bundle to trust instead")
	}
	cert, hasCert := r.settings["client_cert"]
	key, hasKey := r.settings["client_key"]
	switch {
	case hasCert && !hasKey:
		r.problem("client_key", cert.Source, "client_cert needs client_key")
	case !hasCert && hasKey:
		r.problem("client_cert", key.Source, "client_key needs client_cert")
	}
	for _, name := range []string{"ca_bundle", "client_cert", "client_key"} {
		if v, ok := r.settings[name]; ok {
			if p, ok := v.Value.(string); ok && !readable(p) {
				r.problem(name, v.Source, "cannot read "+p)
			}
		}
	}
}
