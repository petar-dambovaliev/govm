package main

import "github.com/gnolang/gno-rs/local_importing/add"

func main() {
	println(add.Add(1, 0))
	println(add.Add(1, 1))
}

// Output:
//1
//2
