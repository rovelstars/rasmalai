// binary_trees: struct-of-arrays form, minDepth 4, maxDepth 16, stretch 17.
function build(t, depth, item) {
    const idx = t.items.length;
    t.items.push(item); t.lefts.push(-1); t.rights.push(-1);
    if (depth > 0) {
        t.lefts[idx] = build(t, depth - 1, item * 2 - 1);
        t.rights[idx] = build(t, depth - 1, item * 2);
    }
    return idx;
}
function check(t, idx) {
    if (idx < 0) return 0;
    return t.items[idx] + check(t, t.lefts[idx]) - check(t, t.rights[idx]);
}
function oneTree(depth, item) {
    const t = { items: [], lefts: [], rights: [] };
    return check(t, build(t, depth, item));
}
let total = oneTree(17, 0);
const ll = { items: [], lefts: [], rights: [] };
total += check(ll, build(ll, 16, 0));
for (let d = 4; d <= 16; d += 2) {
    const iters = 16 > d ? 16 : 8;
    let cs = 0;
    for (let k = 0; k < iters; k++) cs += oneTree(d, k);
    total += cs;
}
console.log('RESULT checksum', total);
