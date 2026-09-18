package fixtures

func choose(x int) int {
	if x > 0 {
		if x > 10 {
			return 2
		}
	} else if x < 0 {
		return -1
	} else {
		return 0
	}
	return 1
}
