// Command tool prints its arguments reversed.
package main

import (
	"fmt"
	"os"
	"strings"

	"example.com/textkit/strutil"
)

func main() {
	fmt.Println(strutil.Reverse(strings.Join(os.Args[1:], " ")))
}
