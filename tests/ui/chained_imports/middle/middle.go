package middle

import "github.com/test/chained_imports/leaf"

func DoubleInc(x int) int {
	return leaf.Inc(leaf.Inc(x))
}

func GetBase() int {
	return leaf.Base
}
