package main

import "github.com/test/cross_pkg_types/math"

func main() {
	println(math.Factor)
	println(math.Multiply(3))
	println(math.Square(4))
	println(math.Multiply(0))
}

// Output:
//10
//30
//16
//0
