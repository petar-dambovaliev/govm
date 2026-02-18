package main

type Point struct {
	X int
	Y int
}

func (p Point) Sum() int {
	return p.X + p.Y
}

func main() {
	p := Point{X: 3, Y: 4}
	println(p.X)
	println(p.Y)
	println(p.Sum())

	p2 := Point{X: 10, Y: 20}
	println(p2.Sum())
}

// Output:
//3
//4
//7
//30
