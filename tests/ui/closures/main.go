package main

func makeCounter() func() int {
	count := 0
	return func() int {
		count++
		return count
	}
}

func main() {
	counter := makeCounter()
	println(counter())
	println(counter())
	println(counter())
}

// Output:
//1
//2
//3
