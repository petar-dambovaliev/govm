package main

func main() {
	ch := make(chan int, 2)
	ch <- 10
	ch <- 20
	close(ch)
	a := <-ch
	b := <-ch
	println(a)
	println(b)
}

// Output:
//10
//20
