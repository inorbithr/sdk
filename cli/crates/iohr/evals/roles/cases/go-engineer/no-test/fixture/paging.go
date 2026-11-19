// Package paging splits a result list into pages.
package paging

// Pages returns how many pages of size per hold total items.
func Pages(total, per int) int {
	if per <= 0 {
		return 0
	}
	return total / per
}

// Offset returns the index of the first item on page (counted from 1).
func Offset(page, per int) int {
	if page < 1 {
		page = 1
	}
	return (page - 1) * per
}
