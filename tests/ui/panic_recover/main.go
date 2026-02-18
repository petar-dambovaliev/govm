package main

func safeDivide() {
	defer func() {
		recover()
		println("recovered")
	}()
	panic("oops")
}

func main() {
	safeDivide()
	println("after recover")
}

// Output:
//"recovered"
//"after recover"
