// binary_trees: struct-of-arrays form, minDepth 4, maxDepth 16, stretch 17.
class Tables {
  final items = <int>[];
  final lefts = <int>[];
  final rights = <int>[];
}

int build(Tables t, int depth, int item) {
  final idx = t.items.length;
  t.items.add(item);
  t.lefts.add(-1);
  t.rights.add(-1);
  if (depth > 0) {
    t.lefts[idx] = build(t, depth - 1, item * 2 - 1);
    t.rights[idx] = build(t, depth - 1, item * 2);
  }
  return idx;
}

int check(Tables t, int idx) {
  if (idx < 0) return 0;
  return t.items[idx] + check(t, t.lefts[idx]) - check(t, t.rights[idx]);
}

int oneTree(int depth, int item) {
  final t = Tables();
  return check(t, build(t, depth, item));
}

void main() {
  var total = oneTree(17, 0);
  final ll = Tables();
  total += check(ll, build(ll, 16, 0));
  for (var d = 4; d <= 16; d += 2) {
    final iters = 16 > d ? 16 : 8;
    var cs = 0;
    for (var k = 0; k < iters; k++) {
      cs += oneTree(d, k);
    }
    total += cs;
  }
  print('RESULT checksum $total');
}
