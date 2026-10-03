// matmul 256x256 integer matrices, ijk loop order.
package main

import "fmt"

const n = 256

func main() {
	a := make([]int64, n*n)
	b := make([]int64, n*n)
	c := make([]int64, n*n)
	for r := 0; r < n; r++ {
		for k := 0; k < n; k++ {
			a[r*n+k] = int64((r + k) % 64)
			b[r*n+k] = int64((r * k) % 64)
		}
	}
	for x := 0; x < n; x++ {
		for y := 0; y < n; y++ {
			var s int64
			for z := 0; z < n; z++ {
				s += a[x*n+z] * b[z*n+y]
			}
			c[x*n+y] = s
		}
	}
	var total int64
	for _, v := range c {
		total += v
	}
	fmt.Println("RESULT checksum", total)
}
