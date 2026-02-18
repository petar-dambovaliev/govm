package main

var globalX int = 100

func main() {
	println(globalX)

	a := 1
	b := 2
	c := a + b
	println(c)

	a = 10
	println(a)

	var d int = 42
	println(d)
}

// Output:
//100
//3
//10
//42
