// spectral-norm N=300, 10 power iterations on AtA.
package main

import (
	"fmt"
	"math"
)

const n = 300

func aElem(i, j int) float64 {
	w := (i+j)*(i+j+1)/2 + i + 1
	return 1.0 / float64(w)
}

func av(x, y []float64) {
	for i := 0; i < n; i++ {
		s := 0.0
		for j := 0; j < n; j++ {
			s += aElem(i, j) * x[j]
		}
		y[i] = s
	}
}

func atv(x, y []float64) {
	for i := 0; i < n; i++ {
		s := 0.0
		for j := 0; j < n; j++ {
			s += aElem(j, i) * x[j]
		}
		y[i] = s
	}
}

func main() {
	u := make([]float64, n)
	v := make([]float64, n)
	tmp := make([]float64, n)
	for i := range u {
		u[i] = 1.0
	}
	for k := 0; k < 10; k++ {
		av(u, tmp)
		atv(tmp, v)
		av(v, tmp)
		atv(tmp, u)
	}
	vBv, vv := 0.0, 0.0
	for j := 0; j < n; j++ {
		vBv += u[j] * v[j]
		vv += v[j] * v[j]
	}
	fmt.Println("RESULT checksum", math.Sqrt(vBv/vv))
}
