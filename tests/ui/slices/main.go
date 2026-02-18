package main

func main() {
	s := []int{10, 20, 30}
	println(len(s))
	println(s[0])
	println(s[1])
	println(s[2])

	s = append(s, 40)
	println(len(s))
	println(s[3])
}

// Output:
//3
//10
//20
//30
//4
//40
