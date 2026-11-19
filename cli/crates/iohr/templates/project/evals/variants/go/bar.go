package main

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

// Bar is what the suite must reach to pass, read from thresholds.toml.
type Bar struct {
	MinPassRate float64
	MaxErrors   int
	Scorers     map[string]float64
}

var (
	tableLine = regexp.MustCompile(`^\[([A-Za-z0-9_.]+)\]$`)
	keyLine   = regexp.MustCompile(`^([A-Za-z0-9_]+)\s*=\s*([+-]?[0-9]+(\.[0-9]+)?)$`)
)

// parseTables reads the small part of TOML the bar uses: `[table]` lines, `key = number` lines and
// `#` comments. The standard library has no TOML reader, and the bar needs nothing more, so
// anything else is refused rather than guessed at.
func parseTables(name, text string) (map[string]map[string]string, error) {
	tables := map[string]map[string]string{}
	var current map[string]string
	for i, line := range strings.Split(text, "\n") {
		if at := strings.IndexByte(line, '#'); at >= 0 {
			line = line[:at]
		}
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		if m := tableLine.FindStringSubmatch(line); m != nil {
			if _, dup := tables[m[1]]; dup {
				return nil, suiteErrorf("%s: not valid TOML (line %d: table [%s] twice)", name, i+1, m[1])
			}
			current = map[string]string{}
			tables[m[1]] = current
			continue
		}
		m := keyLine.FindStringSubmatch(line)
		if m == nil || current == nil {
			return nil, suiteErrorf("%s: not valid TOML (line %d: only `[table]` and `key = number` are read here)", name, i+1)
		}
		if _, dup := current[m[1]]; dup {
			return nil, suiteErrorf("%s: not valid TOML (line %d: key %s twice)", name, i+1, m[1])
		}
		current[m[1]] = m[2]
	}
	return tables, nil
}

func loadBar(path string, names []string) (Bar, error) {
	file := filepath.Base(path)
	data, err := os.ReadFile(path)
	if err != nil {
		return Bar{}, suiteErrorf("cannot read the bar %s: %s", path, strings.TrimPrefix(err.Error(), "open "+path+": "))
	}
	tables, err := parseTables(file, string(data))
	if err != nil {
		return Bar{}, err
	}
	bar, ok := tables["bar"]
	if !ok {
		return Bar{}, suiteErrorf("%s: the [bar] table is missing", file)
	}
	needs := suiteErrorf("%s: [bar] needs min_pass_rate, max_errors and [bar.scorers]", file)
	rate, errRate := strconv.ParseFloat(bar["min_pass_rate"], 64)
	errors, errMax := strconv.Atoi(bar["max_errors"])
	per, hasPer := tables["bar.scorers"]
	if errRate != nil || errMax != nil || !hasPer {
		return Bar{}, needs
	}
	out := Bar{MinPassRate: rate, MaxErrors: errors, Scorers: map[string]float64{}}
	if rate < 0 || rate > 1 {
		return Bar{}, suiteErrorf("%s: every rate is a number from 0 to 1", file)
	}
	for k, v := range per {
		f, _ := strconv.ParseFloat(v, 64)
		if f < 0 || f > 1 {
			return Bar{}, suiteErrorf("%s: every rate is a number from 0 to 1", file)
		}
		out.Scorers[k] = f
	}
	if errors < 0 {
		return Bar{}, suiteErrorf("%s: max_errors cannot be negative", file)
	}
	var unguarded, unknown []string
	for _, n := range names {
		if _, ok := out.Scorers[n]; !ok {
			unguarded = append(unguarded, n)
		}
	}
	for k := range out.Scorers {
		if !contains(names, k) {
			unknown = append(unknown, k)
		}
	}
	sort.Strings(unguarded)
	sort.Strings(unknown)
	if len(unguarded) > 0 {
		return Bar{}, suiteErrorf("%s: no bar for the scorer(s) %s; a scorer with no bar guards nothing", file, strings.Join(unguarded, ", "))
	}
	if len(unknown) > 0 {
		return Bar{}, suiteErrorf("%s: a bar for %s, which is not a scorer", file, strings.Join(unknown, ", "))
	}
	return out, nil
}

func contains(list []string, s string) bool {
	for _, x := range list {
		if x == s {
			return true
		}
	}
	return false
}

// missed lists every line of the bar the report does not reach; empty means the bar is met.
func missed(r Report, bar Bar) []string {
	var out []string
	if r.passRate() < bar.MinPassRate {
		out = append(out, fmt.Sprintf("pass rate %.3f is below %.3f", r.passRate(), bar.MinPassRate))
	}
	if r.errors() > bar.MaxErrors {
		out = append(out, fmt.Sprintf("%d error(s), at most %d allowed", r.errors(), bar.MaxErrors))
	}
	names := make([]string, 0, len(bar.Scorers))
	for n := range bar.Scorers {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, n := range names {
		if mean := r.mean(n); mean < bar.Scorers[n] {
			out = append(out, fmt.Sprintf("scorer %s: mean %.3f is below %.3f", n, mean, bar.Scorers[n]))
		}
	}
	return out
}
