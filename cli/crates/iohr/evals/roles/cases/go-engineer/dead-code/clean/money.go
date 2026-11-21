// Package money holds amounts in cents.
package money

import (
	"fmt"
	"math"
)

// Round returns amount rounded to whole cents, half away from zero.
func Round(amount float64) int64 {
	return int64(math.Round(amount * 100))
}

// Format renders cents as units and cents, such as 12.50.
func Format(cents int64) string {
	sign := ""
	if cents < 0 {
		sign, cents = "-", -cents
	}
	return fmt.Sprintf("%s%d.%02d", sign, cents/100, cents%100)
}
