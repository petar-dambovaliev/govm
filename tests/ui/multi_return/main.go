package main

func divmod(a int, b int) (int, int) {
	return a / b, a % b
}

func main() {
	q, r := divmod(17, 5)
	println(q)
	println(r)

	q2, r2 := divmod(100, 7)
	println(q2)
	println(r2)
}

// Output:
//3
//2
//14
//2
