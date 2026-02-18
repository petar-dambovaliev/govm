package main

func main() {
	ch1 := make(chan int, 1)
	ch2 := make(chan int, 1)

	ch1 <- 42

	select {
	case a := <-ch1:
		println(a)
	case b := <-ch2:
		println(b)
	}
}

// Output:
//42
