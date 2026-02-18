package main

func main() {
	ch := make(chan int, 1)

	select {
	case a := <-ch:
		println(a)
	default:
		println(99)
	}
}

// Output:
//99
