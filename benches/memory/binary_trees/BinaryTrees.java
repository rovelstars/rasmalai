// binary_trees: struct-of-arrays form, minDepth 4, maxDepth 16, stretch 17.
public class BinaryTrees {
    static class Tables {
        long[] items = new long[1024];
        long[] lefts = new long[1024];
        long[] rights = new long[1024];
        int len = 0;

        void grow() {
            int cap = items.length * 2;
            long[] ni = new long[cap];
            long[] nl = new long[cap];
            long[] nr = new long[cap];
            System.arraycopy(items, 0, ni, 0, len);
            System.arraycopy(lefts, 0, nl, 0, len);
            System.arraycopy(rights, 0, nr, 0, len);
            items = ni;
            lefts = nl;
            rights = nr;
        }
    }

    static long build(Tables t, int depth, long item) {
        if (t.len == t.items.length) t.grow();
        int idx = t.len++;
        t.items[idx] = item;
        t.lefts[idx] = -1L;
        t.rights[idx] = -1L;
        if (depth > 0) {
            // Read the child links into temps first: the recursive builds
            // may grow (reallocate) the arrays, so t.lefts/t.rights must
            // be re-read after the calls return, not before.
            long l = build(t, depth - 1, item * 2 - 1);
            long r = build(t, depth - 1, item * 2);
            t.lefts[idx] = l;
            t.rights[idx] = r;
        }
        return idx;
    }

    static long check(Tables t, long idx) {
        if (idx < 0) return 0;
        int i = (int) idx;
        return t.items[i] + check(t, t.lefts[i]) - check(t, t.rights[i]);
    }

    static long oneTree(int depth, long item) {
        Tables t = new Tables();
        return check(t, build(t, depth, item));
    }

    public static void main(String[] args) {
        long total = oneTree(17, 0);
        Tables ll = new Tables();
        total += check(ll, build(ll, 16, 0));
        for (int d = 4; d <= 16; d += 2) {
            int iters = 16 > d ? 16 : 8;
            long cs = 0;
            for (int k = 0; k < iters; k++) cs += oneTree(d, k);
            total += cs;
        }
        System.out.println("RESULT checksum " + total);
    }
}
