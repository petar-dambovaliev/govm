package main

func producer(ch chan int, base int) {
	ch <- base + 1
	ch <- base + 2
}

func main() {
	ch := make(chan int, 4)
	go producer(ch, 0)
	go producer(ch, 10)

	sum := 0
	for i := 0; i < 4; i++ {
		val := <-ch
		sum = sum + val
	}
	println(sum)
}

// Output:
//26
