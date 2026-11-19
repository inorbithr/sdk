package main

import "fmt"

// Result is one case, graded. Err is empty unless the solver failed.
type Result struct {
	ID     string
	Answer string
	Scores map[string]int
	Err    string
}

func (r Result) passed() bool {
	if r.Err != "" {
		return false
	}
	for _, s := range r.Scores {
		if s != 1 {
			return false
		}
	}
	return true
}

// Report is every result and which solver made them.
type Report struct {
	Solver  string
	Stub    bool
	Results []Result
}

func (r Report) passes() int {
	n := 0
	for _, x := range r.Results {
		if x.passed() {
			n++
		}
	}
	return n
}

func (r Report) passRate() float64 { return float64(r.passes()) / float64(len(r.Results)) }

func (r Report) errors() int {
	n := 0
	for _, x := range r.Results {
		if x.Err != "" {
			n++
		}
	}
	return n
}

func (r Report) mean(scorer string) float64 {
	sum := 0
	for _, x := range r.Results {
		sum += x.Scores[scorer]
	}
	return float64(sum) / float64(len(r.Results))
}

// solve calls the solver and turns a panic into an error, so a broken solver is an error case.
func solve(s Solver, c Case) (answer string, err error) {
	defer func() {
		if p := recover(); p != nil {
			err = fmt.Errorf("panic: %v", p)
		}
	}()
	return s.Solve(c)
}

// evaluate runs the solver over the cases and scores each answer. A solver that fails is an
// error case with every score 0, never a pass.
func evaluate(cases []Case, s Solver) Report {
	report := Report{Solver: s.Name(), Stub: s.Stub()}
	for _, c := range cases {
		answer, err := solve(s, c)
		res := Result{ID: c.ID, Answer: answer, Scores: map[string]int{}}
		for _, sc := range scorers {
			if err != nil {
				res.Scores[sc.name] = 0
			} else {
				res.Scores[sc.name] = sc.fn(c, answer)
			}
		}
		if err != nil {
			res.Err = fmt.Sprintf("%T", err)
			res.Answer = ""
		}
		report.Results = append(report.Results, res)
	}
	return report
}
