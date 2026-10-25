typedef struct fx_db fx_db;

int fx_add(int a, int b);
void fx_nop(int x);
double fx_scale(double x, double y);
int fx_fill(int *buf, int n);
int fx_len(const char *s);
int fx_open(const char *name, fx_db **out);
int fx_close(fx_db *db);
int fx_each(int (*cb)(int));
void *fx_alloc(int n);
int fx_flex(const char *fmt, ...);
