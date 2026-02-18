package main

func fib(n int) int {
	if n <= 1 {
		return n
	}
	return fib(n-1) + fib(n-2)
}

func main() {
	println(fib(0))
	println(fib(1))
	println(fib(5))
	println(fib(10))
}

// Output:
//0
//1
//5
//55
