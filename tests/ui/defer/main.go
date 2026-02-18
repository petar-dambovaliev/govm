package main

func printNum(n int) {
	println(n)
}

func main() {
	x := 10
	defer printNum(x)
	x = 20
	println(x)
}

// Output:
//20
//10
