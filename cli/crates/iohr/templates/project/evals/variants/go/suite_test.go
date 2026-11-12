package main

// The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an answer
// is wrong, and the harness refuses a broken dataset or bar instead of grading less.

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func runSuite(args ...string) (int, string, string) {
	var out, errOut bytes.Buffer
	code := suite(args, &out, &errOut, nil)
	return code, out.String(), errOut.String()
}

func write(t *testing.T, name, text string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestTheFixtureSuiteMeetsItsBar(t *testing.T) {
	// The suite opens no connection: it uses no network package, only files and the solver.
	code, out, _ := runSuite()
	if code != 0 || !strings.Contains(out, "BAR MET") {
		t.Fatalf("code %d\n%s", code, out)
	}
}

func TestTheDatasetIsAboutADozenLabelledCasesCoveringEveryLabel(t *testing.T) {
	cases, err := loadCases(defaultDataset)
	if err != nil || len(cases) != 12 {
		t.Fatalf("%d cases, %v", len(cases), err)
	}
	seen := map[string]bool{}
	for _, c := range cases {
		seen[c.Expected] = true
	}
	for _, l := range labels {
		if !seen[l] {
			t.Errorf("no case for the label %s", l)
		}
	}
	if len(seen) != len(labels) {
		t.Errorf("labels in the dataset %v, known %v", seen, labels)
	}
}

func TestTheReportSaysTheSolverIsAStubAndHoldsNoInputs(t *testing.T) {
	code, out, _ := runSuite("--json")
	var report map[string]any
	if err := json.Unmarshal([]byte(out), &report); err != nil || code != 0 {
		t.Fatalf("%d %v\n%s", code, err, out)
	}
	if report["stub"] != true || !strings.HasPrefix(report["solver"].(string), "stub-") ||
		report["bar_met"] != true || report["pass_rate"] != float64(1) {
		t.Fatalf("report %v", report)
	}
	cases, _ := loadCases(defaultDataset)
	for _, c := range cases {
		if strings.Contains(out, c.Input) {
			t.Errorf("the report holds an input: %s", c.Input)
		}
	}
	if code2, out2, _ := runSuite("--json"); code2 != code || out2 != out {
		t.Error("two runs gave different reports")
	}
}

// datasetWith copies the dataset with the first case changed and returns the path and its id.
func datasetWith(t *testing.T, change func(row map[string]any)) (string, string) {
	t.Helper()
	data, err := os.ReadFile(defaultDataset)
	if err != nil {
		t.Fatal(err)
	}
	var lines []string
	id := ""
	for i, line := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		if i == 0 {
			var row map[string]any
			if err := json.Unmarshal([]byte(line), &row); err != nil {
				t.Fatal(err)
			}
			change(row)
			id = row["id"].(string)
			b, _ := json.Marshal(row)
			line = string(b)
		}
		lines = append(lines, line)
	}
	return write(t, "cases.jsonl", strings.Join(lines, "\n")+"\n"), id
}

func TestLoweringOneExpectedAnswerFailsTheSuiteAndNamesTheCase(t *testing.T) {
	path, id := datasetWith(t, func(row map[string]any) { row["expected"] = "other" })
	code, out, _ := runSuite("--dataset", path)
	if code != 1 || !strings.Contains(out, "BAR MISSED") || !strings.Contains(out, "FAILED "+id) {
		t.Fatalf("code %d\n%s", code, out)
	}
}

type chatty struct{ stubSolver }

func (c chatty) Name() string { return "stub-chatty" }
func (c chatty) Solve(cs Case) (string, error) {
	a, err := c.stubSolver.Solve(cs)
	return "The label is " + a + ".", err
}

type broken struct{ stubSolver }

func (broken) Solve(Case) (string, error) { return "", errors.New("provider down") }

type panicking struct{ stubSolver }

func (panicking) Solve(Case) (string, error) { panic("boom") }

func TestAChattyAnswerFailsTheFormatScorerEvenWhenItIsRight(t *testing.T) {
	cases, _ := loadCases(defaultDataset)
	bar, err := loadBar(defaultBar, scorerNames())
	if err != nil {
		t.Fatal(err)
	}
	report := evaluate(cases, chatty{})
	if report.mean("format") != 0 || len(missed(report, bar)) == 0 {
		t.Fatalf("format mean %v", report.mean("format"))
	}
}

func TestASolverThatFailsIsAnErrorAndNeverAPass(t *testing.T) {
	cases, _ := loadCases(defaultDataset)
	bar, _ := loadBar(defaultBar, scorerNames())
	for _, s := range []Solver{broken{}, panicking{}} {
		report := evaluate(cases, s)
		if report.errors() != 12 || report.passRate() != 0 {
			t.Fatalf("%T: errors %d, pass rate %v", s, report.errors(), report.passRate())
		}
		if !slices.ContainsFunc(missed(report, bar), func(m string) bool { return strings.Contains(m, "error") }) {
			t.Fatalf("%T: the bar did not name the errors", s)
		}
	}
}

func TestABrokenDatasetStopsTheRun(t *testing.T) {
	row := `{"id": "a", "input": "i", "expected": "bug"}` + "\n"
	for text, why := range map[string]string{
		row + row: "twice",
		`{"id": "a", "input": "", "expected": "bug"}` + "\n": "input",
		"\n":         "no cases",
		"not json\n": "not JSON",
	} {
		code, _, errOut := runSuite("--dataset", write(t, "c.jsonl", text))
		if code != 2 || !strings.Contains(errOut, why) {
			t.Errorf("%s: code %d, %q", why, code, errOut)
		}
	}
}

func TestAScorerWithNoBarStopsTheRun(t *testing.T) {
	bar := write(t, "t.toml", "[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n")
	code, _, errOut := runSuite("--bar", bar)
	if code != 2 || !strings.Contains(errOut, "no bar for the scorer(s) format") {
		t.Fatalf("code %d, %q", code, errOut)
	}
}

func TestABarThatIsNotUnderstoodStopsTheRun(t *testing.T) {
	for _, text := range []string{"[bar]\nmin_pass_rate = high\n", "min_pass_rate = 1\n", "[bar]\nmin_pass_rate = 2\nmax_errors = 0\n[bar.scorers]\nexact = 1\nformat = 1\n"} {
		if code, _, _ := runSuite("--bar", write(t, "t.toml", text)); code != 2 {
			t.Errorf("%q: code %d", text, code)
		}
	}
}

func TestAMissingBarOrDatasetStopsTheRun(t *testing.T) {
	if code, _, _ := runSuite("--bar", "/nonexistent/t.toml"); code != 2 {
		t.Error("missing bar")
	}
	if code, _, _ := runSuite("--dataset", "/nonexistent/c.jsonl"); code != 2 {
		t.Error("missing dataset")
	}
}

func TestExactIgnoresCaseAndSpaceOnlyAndFormatWantsOneBareLabel(t *testing.T) {
	c := Case{"x", "i", "billing"}
	if exact(c, " Billing\n") != 1 || exact(c, "bug") != 0 {
		t.Error("exact")
	}
	if formatOK(c, "billing") != 1 {
		t.Error("format of a bare label")
	}
	for _, bad := range []string{"Billing", "billing.", "billing, bug", ""} {
		if formatOK(c, bad) != 0 {
			t.Errorf("format accepted %q", bad)
		}
	}
}

func TestEveryScorerHasABar(t *testing.T) {
	bar, err := loadBar(defaultBar, scorerNames())
	if err != nil || len(bar.Scorers) != len(scorers) {
		t.Fatalf("%v %v", bar, err)
	}
}
