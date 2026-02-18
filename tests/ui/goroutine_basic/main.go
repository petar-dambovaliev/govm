package main

func worker(ch chan int, val int) {
	ch <- val * 2
}

func main() {
	ch := make(chan int)
	go worker(ch, 21)
	result := <-ch
	println(result)
}

// Output:
//42
