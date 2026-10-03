// json_stress: ~1.5 MB document of 20000 flat objects,
// 5 rounds of parse + compact reserialize, checksum = byte sum of output.
package main

import (
	"encoding/json"
	"fmt"
	"strings"
)

type Obj struct {
	ID    int      `json:"id"`
	Name  string   `json:"name"`
	Tags  []string `json:"tags"`
	Score float64  `json:"score"`
	Ok    bool     `json:"ok"`
}

func main() {
	const piece = `{"id": 7, "name": "name-7", "tags": ["a", "b", "c"], "score": 1.5, "ok": true}`
	chunkParts := make([]string, 500)
	for i := range chunkParts {
		chunkParts[i] = piece
	}
	chunk := strings.Join(chunkParts, ",")
	docParts := make([]string, 40)
	for i := range docParts {
		docParts[i] = chunk
	}
	doc := "[" + strings.Join(docParts, ",") + "]"
	fmt.Println("RESULT doclen", len(doc))

	var charsum int64
	for r := 0; r < 5; r++ {
		var arr []Obj
		if err := json.Unmarshal([]byte(doc), &arr); err != nil {
			panic(err)
		}
		back, err := json.Marshal(arr)
		if err != nil {
			panic(err)
		}
		for _, b := range back {
			charsum += int64(b)
		}
	}
	fmt.Println("RESULT checksum", charsum)
}
