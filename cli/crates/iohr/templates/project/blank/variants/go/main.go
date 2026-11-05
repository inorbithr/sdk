package main

import "fmt"

func hello() string {
	return "hello from @@NAME@@"
}

func main() {
	fmt.Println(hello())
}
