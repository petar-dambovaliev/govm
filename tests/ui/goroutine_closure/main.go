package main

func multiply(ch chan int, x int) {
	ch <- x * 10
}

func main() {
	ch := make(chan int)
	go multiply(ch, 5)
	result := <-ch
	println(result)
}

// Output:
//50
