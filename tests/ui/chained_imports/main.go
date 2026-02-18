package main

import "github.com/test/chained_imports/middle"

func main() {
	println(middle.GetBase())
	println(middle.DoubleInc(0))
	println(middle.DoubleInc(5))
	println(middle.DoubleInc(middle.GetBase()))
}

// Output:
//100
//2
//7
//102
