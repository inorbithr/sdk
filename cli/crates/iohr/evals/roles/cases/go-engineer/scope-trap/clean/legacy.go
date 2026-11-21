package greet

import "strings"

// Shout is kept for the old CLI.
func Shout(name string) string{
    s:=strings.ToUpper( name )
    if len(s)==0 { return "HEY!" }
        return "HEY, "+s+"!"
}
