// binary_trees: struct-of-arrays form, minDepth 4, maxDepth 16, stretch 17.
#include <stdio.h>
#include <stdlib.h>

typedef struct { long *items, *lefts, *rights; long len, cap; } Tables;

static void tinit(Tables *t) { t->len = 0; t->cap = 1024; t->items = malloc(sizeof(long) * 1024); t->lefts = malloc(sizeof(long) * 1024); t->rights = malloc(sizeof(long) * 1024); }
static void tfree(Tables *t) { free(t->items); free(t->lefts); free(t->rights); }

static long build(Tables *t, int depth, long item) {
    if (t->len == t->cap) {
        t->cap *= 2;
        t->items = realloc(t->items, sizeof(long) * t->cap);
        t->lefts = realloc(t->lefts, sizeof(long) * t->cap);
        t->rights = realloc(t->rights, sizeof(long) * t->cap);
    }
    long idx = t->len++;
    t->items[idx] = item; t->lefts[idx] = -1; t->rights[idx] = -1;
    if (depth > 0) {
        t->lefts[idx] = build(t, depth - 1, item * 2 - 1);
        t->rights[idx] = build(t, depth - 1, item * 2);
    }
    return idx;
}

static long check(Tables *t, long idx) {
    if (idx < 0) return 0;
    return t->items[idx] + check(t, t->lefts[idx]) - check(t, t->rights[idx]);
}

static long one_tree(int depth, long item) {
    Tables t; tinit(&t);
    long root = build(&t, depth, item);
    long c = check(&t, root);
    tfree(&t);
    return c;
}

int main(void) {
    long total = one_tree(17, 0);
    Tables ll; tinit(&ll);
    long root = build(&ll, 16, 0);
    total += check(&ll, root);
    for (int d = 4; d <= 16; d += 2) {
        int iters = (16 > d) ? 16 : 8;
        long cs = 0;
        for (int k = 0; k < iters; k++) cs += one_tree(d, k);
        total += cs;
    }
    tfree(&ll);
    printf("RESULT checksum %ld\n", total);
    return 0;
}
