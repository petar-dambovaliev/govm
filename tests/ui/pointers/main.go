package main

func main() {
	x := 10
	y := &x
	println(*y)

	*y = 20
	println(x)
}

// Output:
//10
//20
