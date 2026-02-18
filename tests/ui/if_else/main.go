package main

func main() {
	x := 10

	if x > 5 {
		println("greater")
	} else {
		println("not greater")
	}

	if x == 10 {
		println("equal")
	}

	if x < 5 {
		println("should not print")
	} else if x < 15 {
		println("in range")
	} else {
		println("too big")
	}

	y := x + 1
	if y > 10 {
		println(y)
	}
}

// Output:
//"greater"
//"equal"
//"in range"
//11
