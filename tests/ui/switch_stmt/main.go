package main

func main() {
	day := 3
	switch day {
	case 1:
		println("Monday")
	case 2:
		println("Tuesday")
	case 3:
		println("Wednesday")
	default:
		println("Other")
	}

	x := 42
	switch {
	case x > 100:
		println("big")
	case x > 10:
		println("medium")
	default:
		println("small")
	}
}

// Output:
//"Wednesday"
//"medium"
