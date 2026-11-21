// Package money holds amounts in cents.
package money

import "math"

// Round returns amount rounded to whole cents, half away from zero.
func Round(amount float64) int64 {
	return int64(math.Round(amount * 100))
}

// roundHalfEven was the rounding before Round; nothing calls it since the ledger moved.
func roundHalfEven(amount float64) int64 {
	return int64(math.RoundToEven(amount * 100))
}
