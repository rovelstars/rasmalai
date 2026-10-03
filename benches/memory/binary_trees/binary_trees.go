// binary_trees: struct-of-arrays form, minDepth 4, maxDepth 16, stretch 17.
package main

import "fmt"

type Tables struct {
	items  []int64
	lefts  []int64
	rights []int64
}

func build(t *Tables, depth int, item int64) int64 {
	idx := int64(len(t.items))
	t.items = append(t.items, item)
	t.lefts = append(t.lefts, -1)
	t.rights = append(t.rights, -1)
	if depth > 0 {
		t.lefts[idx] = build(t, depth-1, item*2-1)
		t.rights[idx] = build(t, depth-1, item*2)
	}
	return idx
}

func check(t *Tables, idx int64) int64 {
	if idx < 0 {
		return 0
	}
	return t.items[idx] + check(t, t.lefts[idx]) - check(t, t.rights[idx])
}

func oneTree(depth int, item int64) int64 {
	t := &Tables{}
	return check(t, build(t, depth, item))
}

func main() {
	total := oneTree(17, 0)
	ll := &Tables{}
	total += check(ll, build(ll, 16, 0))
	for d := 4; d <= 16; d += 2 {
		iters := 8
		if 16 > d {
			iters = 16
		}
		var cs int64
		for k := 0; k < iters; k++ {
			cs += oneTree(d, int64(k))
		}
		total += cs
	}
	fmt.Println("RESULT checksum", total)
}
