package main

func main() {
	m := map[string]int{"x": 10, "y": 20}
	m["z"] = 30
	println(len(m))
}

// Output:
//3
