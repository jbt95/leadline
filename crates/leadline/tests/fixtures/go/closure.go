package fixtures

func outer(x int) int {
	double := func(n int) int {
		return n * 2
	}
	return double(x)
}
