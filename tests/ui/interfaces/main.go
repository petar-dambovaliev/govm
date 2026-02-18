package main

type Shape interface {
	Area() int
}

type Rect struct {
	W int
	H int
}

func (r Rect) Area() int {
	return r.W * r.H
}

func printArea(s Shape) {
	println(s.Area())
}

func main() {
	r := Rect{W: 3, H: 4}
	println(r.Area())
	printArea(r)
}

// Output:
//12
//12
