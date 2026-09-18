package fixtures

type Counter struct {
	n int
}

func (c *Counter) add(delta int) int {
	c.n += delta
	return c.n
}
