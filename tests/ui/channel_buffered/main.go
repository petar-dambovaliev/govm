package main

func main() {
	ch := make(chan int, 2)
	ch <- 42
	ch <- 99
	a := <-ch
	b := <-ch
	println(a)
	println(b)
}

// Output:
//42
//99
