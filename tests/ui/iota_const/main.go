package main

const (
	Red   = iota
	Green
	Blue
)

const (
	A = 10
	B = 20
	C = A + B
)

func main() {
	println(Red)
	println(Green)
	println(Blue)
	println(A)
	println(B)
	println(C)
}

// Output:
//0
//1
//2
//10
//20
//30
