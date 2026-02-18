package main

func main() {
	ch := make(chan int)
	go func() {
		ch <- 100
	}()
	val := <-ch
	println(val)
}

// Output:
//100
