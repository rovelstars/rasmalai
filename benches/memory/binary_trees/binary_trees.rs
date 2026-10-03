// binary_trees: struct-of-arrays form, minDepth 4, maxDepth 16, stretch 17.
struct Tables {
    items: Vec<i64>,
    lefts: Vec<i64>,
    rights: Vec<i64>,
}

fn build(t: &mut Tables, depth: i32, item: i64) -> i64 {
    let idx = t.items.len() as i64;
    t.items.push(item);
    t.lefts.push(-1);
    t.rights.push(-1);
    if depth > 0 {
        let l = build(t, depth - 1, item * 2 - 1);
        let r = build(t, depth - 1, item * 2);
        t.lefts[idx as usize] = l;
        t.rights[idx as usize] = r;
    }
    idx
}

fn check(t: &Tables, idx: i64) -> i64 {
    if idx < 0 {
        return 0;
    }
    let i = idx as usize;
    t.items[i] + check(t, t.lefts[i]) - check(t, t.rights[i])
}

fn one_tree(depth: i32, item: i64) -> i64 {
    let mut t = Tables { items: Vec::new(), lefts: Vec::new(), rights: Vec::new() };
    let root = build(&mut t, depth, item);
    check(&t, root)
}

fn main() {
    let mut total = one_tree(17, 0);
    let mut ll = Tables { items: Vec::new(), lefts: Vec::new(), rights: Vec::new() };
    let root = build(&mut ll, 16, 0);
    total += check(&ll, root);
    let mut d = 4;
    while d <= 16 {
        let iters = if 16 > d { 16 } else { 8 };
        let mut cs = 0;
        for k in 0..iters {
            cs += one_tree(d, k);
        }
        total += cs;
        d += 2;
    }
    println!("RESULT checksum {}", total);
}
