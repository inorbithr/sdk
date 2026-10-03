package inorbit

import (
	"encoding/json"
	"fmt"
	"strconv"
)

// Int64 is a 64-bit integer the API carries as a decimal string, so no value above
// 2^53 loses precision in a JSON reader. It reads a decimal string or a JSON number,
// and writes a decimal string.
type Int64 int64

// String returns the decimal form.
func (n Int64) String() string {
	return strconv.FormatInt(int64(n), 10)
}

// MarshalJSON writes the value as the API reads it: a decimal string.
func (n Int64) MarshalJSON() ([]byte, error) {
	return []byte(strconv.Quote(n.String())), nil
}

// UnmarshalJSON reads a decimal string or a JSON number.
func (n *Int64) UnmarshalJSON(data []byte) error {
	if string(data) == "null" {
		return nil
	}
	text := string(data)
	if len(data) > 0 && data[0] == '"' {
		var s string
		if err := json.Unmarshal(data, &s); err != nil {
			return err
		}
		text = s
	}
	v, err := strconv.ParseInt(text, 10, 64)
	if err != nil {
		return fmt.Errorf("not a 64-bit integer: %s", data)
	}
	*n = Int64(v)
	return nil
}
