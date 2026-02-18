package main

func add(a int, b int) int {
	return a + b
}

func swap(a int, b int) (int, int) {
	return b, a
}

func factorial(n int) int {
	if n <= 1 {
		return 1
	}
	return n * factorial(n-1)
}

func main() {
	println(add(3, 4))

	x, y := swap(1, 2)
	println(x)
	println(y)

	println(factorial(5))
}

// Output:
//7
//2
//1
//120
