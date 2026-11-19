// @@NAME@@: an offline eval suite with a pass bar. Standard library only, no network.
//
// Grade the solver on the dataset and check the pass bar. Exit 0 when the bar is met, 1 when it
// is missed, 2 when the dataset or the bar is wrong. The report says which solver ran and
// whether it is a stub. It holds case ids and scores only; the inputs and answers stay out of it
// (they can be customer data), unless you ask for --show, which prints the failed cases to your
// terminal and nowhere else.
package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"math"
	"os"
	"strings"
)

type jsonResult struct {
	ID     string         `json:"id"`
	Passed bool           `json:"passed"`
	Scores map[string]int `json:"scores"`
	Error  *string        `json:"error"`
}

type jsonReport struct {
	Solver   string             `json:"solver"`
	Stub     bool               `json:"stub"`
	Cases    int                `json:"cases"`
	PassRate float64            `json:"pass_rate"`
	Errors   int                `json:"errors"`
	Scorers  map[string]float64 `json:"scorers"`
	BarMet   bool               `json:"bar_met"`
	Missed   []string           `json:"missed"`
	Results  []jsonResult       `json:"results"`
}

func round4(x float64) float64 { return math.Round(x*1e4) / 1e4 }

func toJSON(r Report, bar []string) jsonReport {
	out := jsonReport{
		Solver: r.Solver, Stub: r.Stub, Cases: len(r.Results), PassRate: round4(r.passRate()),
		Errors: r.errors(), Scorers: map[string]float64{}, BarMet: len(bar) == 0, Missed: bar,
	}
	if out.Missed == nil {
		out.Missed = []string{}
	}
	for _, n := range scorerNames() {
		out.Scorers[n] = round4(r.mean(n))
	}
	for _, x := range r.Results {
		j := jsonResult{ID: x.ID, Passed: x.passed(), Scores: x.Scores}
		if x.Err != "" {
			e := x.Err
			j.Error = &e
		}
		out.Results = append(out.Results, j)
	}
	return out
}

// suite runs the whole thing and returns the exit code; solver is nil for the stub.
func suite(args []string, stdout, stderr io.Writer, solver Solver) int {
	fs := flag.NewFlagSet("@@NAME@@", flag.ContinueOnError)
	fs.SetOutput(stderr)
	dataset := fs.String("dataset", defaultDataset, "JSONL cases (id, input, expected)")
	barPath := fs.String("bar", defaultBar, "the pass bar (thresholds.toml)")
	asJSON := fs.Bool("json", false, "print the report as JSON")
	show := fs.Bool("show", false, "print each failed case's input, expected and answer")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	if solver == nil {
		solver = stubSolver{}
	}
	cases, err := loadCases(*dataset)
	var bar Bar
	if err == nil {
		bar, err = loadBar(*barPath, scorerNames())
	}
	if err != nil {
		var se *suiteError
		if !errors.As(err, &se) {
			panic(err)
		}
		fmt.Fprintf(stderr, "suite error: %v\n", err)
		return 2
	}
	report := evaluate(cases, solver)
	barMissed := missed(report, bar)
	if *asJSON {
		data, _ := json.MarshalIndent(toJSON(report, barMissed), "", "  ")
		fmt.Fprintln(stdout, string(data))
	} else {
		label := ""
		if report.Stub {
			label = "STUB solver, not a model: "
		}
		fmt.Fprintf(stdout, "%s%s: %d/%d cases pass\n", label, report.Solver, report.passes(), len(report.Results))
		for _, n := range scorerNames() {
			fmt.Fprintf(stdout, "  %s: mean %.3f (bar %.3f)\n", n, report.mean(n), bar.Scorers[n])
		}
		fmt.Fprintf(stdout, "  pass rate %.3f (bar %.3f), errors %d (at most %d)\n", report.passRate(), bar.MinPassRate, report.errors(), bar.MaxErrors)
		for _, r := range report.Results {
			if r.passed() {
				continue
			}
			why := r.Err
			if why == "" {
				var zero []string
				for _, n := range scorerNames() {
					if r.Scores[n] == 0 {
						zero = append(zero, n)
					}
				}
				why = strings.Join(zero, ", ")
			}
			fmt.Fprintf(stdout, "  FAILED %s: %s\n", r.ID, why)
		}
		if len(barMissed) == 0 {
			fmt.Fprintln(stdout, "BAR MET")
		} else {
			fmt.Fprintln(stdout, "BAR MISSED: "+strings.Join(barMissed, "; "))
		}
	}
	if *show {
		byID := map[string]Case{}
		for _, c := range cases {
			byID[c.ID] = c
		}
		for _, r := range report.Results {
			if !r.passed() {
				c := byID[r.ID]
				fmt.Fprintf(stderr, "  %s: input=%q expected=%q answer=%q\n", r.ID, c.Input, c.Expected, r.Answer)
			}
		}
	}
	if len(barMissed) != 0 {
		return 1
	}
	return 0
}

func main() { os.Exit(suite(os.Args[1:], os.Stdout, os.Stderr, nil)) }
