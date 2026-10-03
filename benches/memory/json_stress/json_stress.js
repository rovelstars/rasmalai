// json_stress: ~1.5 MB document of 20000 flat objects,
// 5 rounds of JSON.parse + JSON.stringify, checksum = byte sum of output.
const piece = '{"id": 7, "name": "name-7", "tags": ["a", "b", "c"], "score": 1.5, "ok": true}';
const chunk = Array.from({ length: 500 }, () => piece).join(',');
const doc = '[' + Array.from({ length: 40 }, () => chunk).join(',') + ']';
console.log('RESULT doclen', doc.length);

let charsum = 0;
for (let r = 0; r < 5; r++) {
    const back = JSON.stringify(JSON.parse(doc));
    for (let i = 0; i < back.length; i++) charsum += back.charCodeAt(i);
}
console.log('RESULT checksum', charsum);
