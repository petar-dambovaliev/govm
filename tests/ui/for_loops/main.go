package main

func main() {
	sum := 0
	for i := 0; i < 5; i++ {
		sum = sum + i
	}
	println(sum)

	j := 1
	for j < 10 {
		j = j * 2
	}
	println(j)

	count := 0
	for i := 0; i < 10; i++ {
		if i == 3 {
			continue
		}
		if i == 7 {
			break
		}
		count++
	}
	println(count)
}

// Output:
//10
//16
//6
