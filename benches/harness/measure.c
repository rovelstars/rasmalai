/* measure.c - zero-overhead single-process measurement helper.
 *
 * Usage: measure <stdout-file> <command> [args...]
 *
 * fork()s directly from this ~15 KB static-ish binary and execvp()s the
 * target, then wait4()s the child and reports the child's own rusage.
 * Because the parent never maps a heap, interpreter runtime, or shared
 * libraries beyond libc, no copy-on-write pages inflate the child's
 * ru_maxrss. Exit code is always 0 unless invocation itself failed; the
 * child's status is reported in the JSON payload.
 *
 * Output: one JSON line on stdout:
 *   {"elapsed_ns":N,"max_rss_bytes":N,"user_time_ns":N,"sys_time_ns":N,"exit_code":N}
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static long long timespec_ns(const struct timespec *t)
{
    return (long long)t->tv_sec * 1000000000LL + (long long)t->tv_nsec;
}

static long long timeval_ns(const struct timeval *t)
{
    return (long long)t->tv_sec * 1000000000LL + (long long)t->tv_usec * 1000LL;
}

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr, "usage: measure <stdout-file> <command> [args...]\n");
        return 2;
    }
    const char *out = argv[1];
    char **cmd = &argv[2];

    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    pid_t pid = fork();
    if (pid < 0) {
        perror("fork");
        return 2;
    }
    if (pid == 0) {
        FILE *f = freopen(out, "w", stdout);
        if (f == NULL)
            _exit(127);
        execvp(cmd[0], cmd);
        _exit(127);
    }
    int status;
    struct rusage ru;
    if (wait4(pid, &status, 0, &ru) < 0) {
        perror("wait4");
        return 2;
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);

    long exit_code;
    if (WIFEXITED(status))
        exit_code = WEXITSTATUS(status);
    else if (WIFSIGNALED(status))
        exit_code = 128 + WTERMSIG(status);
    else
        exit_code = 127;

#ifdef __APPLE__
    long long max_rss_bytes = (long long)ru.ru_maxrss;
#else
    long long max_rss_bytes = (long long)ru.ru_maxrss * 1024LL;
#endif

    printf("{\"elapsed_ns\":%lld,\"max_rss_bytes\":%lld,"
           "\"user_time_ns\":%lld,\"sys_time_ns\":%lld,\"exit_code\":%ld}\n",
           timespec_ns(&t1) - timespec_ns(&t0), max_rss_bytes,
           timeval_ns(&ru.ru_utime), timeval_ns(&ru.ru_stime), exit_code);
    return 0;
}
