// json_stress: generate a ~1.5 MB document of 20000 flat objects,
// 5 rounds of parse + compact reserialize, checksum = byte sum of output.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef enum { V_INT, V_NUM, V_STR, V_BOOL, V_NULL, V_ARR, V_OBJ } VType;
typedef struct Val Val;
typedef struct { char *key; Val *val; } Pair;
struct Val {
    VType t;
    long ival;
    double fval;
    char *sval;
    int bval;
    Val **items; long nitems;
    Pair *pairs; long npairs;
};

static const char *p;

static void skip_ws(void) { while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r') p++; }

static Val *new_val(VType t) { Val *v = calloc(1, sizeof(Val)); v->t = t; return v; }

static Val *parse_val(void);
static char *parse_str(void) {
    p++; // opening quote
    const char *s = p;
    while (*p != '"') { if (*p == '\\') p++; p++; }
    size_t n = p - s;
    char *o = malloc(n + 1);
    memcpy(o, s, n); o[n] = 0;
    p++; // closing quote
    return o;
}

static Val *parse_val(void) {
    skip_ws();
    if (*p == '{') {
        p++;
        Val *v = new_val(V_OBJ);
        skip_ws();
        if (*p != '}') {
            for (;;) {
                skip_ws();
                char *k = parse_str();
                skip_ws(); p++; // colon
                Val *c = parse_val();
                v->pairs = realloc(v->pairs, sizeof(Pair) * (v->npairs + 1));
                v->pairs[v->npairs].key = k;
                v->pairs[v->npairs].val = c;
                v->npairs++;
                skip_ws();
                if (*p == ',') { p++; continue; }
                break;
            }
        }
        skip_ws(); p++; // closing brace
        return v;
    }
    if (*p == '[') {
        p++;
        Val *v = new_val(V_ARR);
        skip_ws();
        if (*p != ']') {
            for (;;) {
                Val *c = parse_val();
                v->items = realloc(v->items, sizeof(Val *) * (v->nitems + 1));
                v->items[v->nitems++] = c;
                skip_ws();
                if (*p == ',') { p++; continue; }
                break;
            }
        }
        skip_ws(); p++; // closing bracket
        return v;
    }
    if (*p == '"') { Val *v = new_val(V_STR); v->sval = parse_str(); return v; }
    if (*p == 't') { p += 4; Val *v = new_val(V_BOOL); v->bval = 1; return v; }
    if (*p == 'f') { p += 5; Val *v = new_val(V_BOOL); v->bval = 0; return v; }
    if (*p == 'n') { p += 4; return new_val(V_NULL); }
    const char *s = p;
    int is_float = 0;
    if (*p == '-') p++;
    while (*p >= '0' && *p <= '9') p++;
    if (*p == '.') { is_float = 1; p++; while (*p >= '0' && *p <= '9') p++; }
    if (*p == 'e' || *p == 'E') { is_float = 1; p++; if (*p == '+' || *p == '-') p++; while (*p >= '0' && *p <= '9') p++; }
    Val *v = new_val(is_float ? V_NUM : V_INT);
    if (is_float) v->fval = strtod(s, NULL); else v->ival = strtol(s, NULL, 10);
    return v;
}

static void free_val(Val *v) {
    if (!v) return;
    if (v->t == V_STR) free(v->sval);
    for (long i = 0; i < v->nitems; i++) free_val(v->items[i]);
    free(v->items);
    for (long i = 0; i < v->npairs; i++) { free(v->pairs[i].key); free_val(v->pairs[i].val); }
    free(v->pairs);
    free(v);
}

typedef struct { char *buf; size_t len, cap; } Out;
static void emit(Out *o, const char *s, size_t n) {
    if (o->len + n + 1 > o->cap) { o->cap = (o->len + n + 1) * 2 + 64; o->buf = realloc(o->buf, o->cap); }
    memcpy(o->buf + o->len, s, n);
    o->len += n;
}
static void emit_c(Out *o, char c) { emit(o, &c, 1); }

static void ser(Out *o, Val *v) {
    char tmp[32];
    switch (v->t) {
    case V_INT: snprintf(tmp, sizeof tmp, "%ld", v->ival); emit(o, tmp, strlen(tmp)); break;
    case V_NUM: snprintf(tmp, sizeof tmp, "%g", v->fval); emit(o, tmp, strlen(tmp)); break;
    case V_STR: emit_c(o, '"'); emit(o, v->sval, strlen(v->sval)); emit_c(o, '"'); break;
    case V_BOOL: emit(o, v->bval ? "true" : "false", v->bval ? 4 : 5); break;
    case V_NULL: emit(o, "null", 4); break;
    case V_ARR:
        emit_c(o, '[');
        for (long i = 0; i < v->nitems; i++) { if (i) emit_c(o, ','); ser(o, v->items[i]); }
        emit_c(o, ']');
        break;
    case V_OBJ:
        emit_c(o, '{');
        for (long i = 0; i < v->npairs; i++) {
            if (i) emit_c(o, ',');
            emit_c(o, '"'); emit(o, v->pairs[i].key, strlen(v->pairs[i].key)); emit_c(o, '"');
            emit_c(o, ':');
            ser(o, v->pairs[i].val);
        }
        emit_c(o, '}');
        break;
    }
}

// dynamic string builder for the doc
typedef struct { char *buf; size_t len, cap; } Str;
static void sadd(Str *s, const char *t, size_t n) {
    if (s->len + n + 1 > s->cap) { s->cap = (s->len + n + 1) * 2 + 64; s->buf = realloc(s->buf, s->cap); }
    memcpy(s->buf + s->len, t, n);
    s->len += n;
}

int main(void) {
    const char *piece = "{\"id\": 7, \"name\": \"name-7\", \"tags\": [\"a\", \"b\", \"c\"], \"score\": 1.5, \"ok\": true}";
    size_t plen = strlen(piece);
    Str chunk = {0};
    for (int i = 0; i < 500; i++) {
        if (i) sadd(&chunk, ",", 1);
        sadd(&chunk, piece, plen);
    }
    Str doc = {0};
    sadd(&doc, "[", 1);
    for (int c = 0; c < 40; c++) {
        if (c) sadd(&doc, ",", 1);
        sadd(&doc, chunk.buf, chunk.len);
    }
    sadd(&doc, "]", 1);
    doc.buf[doc.len] = 0;
    printf("RESULT doclen %zu\n", doc.len);

    long long charsum = 0;
    for (int r = 0; r < 5; r++) {
        p = doc.buf;
        Val *v = parse_val();
        Out o = {0};
        ser(&o, v);
        for (size_t i = 0; i < o.len; i++) charsum += (unsigned char)o.buf[i];
        free(o.buf);
        free_val(v);
    }
    printf("RESULT checksum %lld\n", charsum);
    free(chunk.buf);
    free(doc.buf);
    return 0;
}
