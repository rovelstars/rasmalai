#include "host_cabi.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <unistd.h>

static int read_all(int fd, uint8_t* buf, int len) {
    int got = 0;
    while (got < len) {
        int n = (int)read(fd, buf + got, (size_t)(len - got));
        if (n <= 0) return -1;
        got += n;
    }
    return 0;
}

static uint32_t u32_at(const uint8_t* p, int at) {
    uint32_t v;
    memcpy(&v, p + at, 4);
    return v;
}

static void put_u32(uint8_t* p, int at, uint32_t v) { memcpy(p + at, &v, 4); }

int main(void) {
    char path[128];
    snprintf(path, sizeof(path), "/tmp/rnx-surface-probe-%d.sock", (int)getpid());
    unlink(path);

    int server = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, path, sizeof(addr.sun_path) - 1);
    if (bind(server, (struct sockaddr*)&addr, sizeof(addr)) != 0 || listen(server, 1) != 0) {
        printf("server setup failed\n");
        return 1;
    }

    pid_t child = fork();
    if (child == 0) {
        int fd = accept(server, NULL, NULL);
        uint8_t hello[20];
        if (read_all(fd, hello, 20) != 0) _exit(2);
        uint32_t hello_id = u32_at(hello, 16);
        uint8_t header[32];
        if (read_all(fd, header, 32) != 0) _exit(3);
        uint32_t frame_id = u32_at(header, 8);
        uint32_t list_len = u32_at(header, 20);
        uint32_t packet_len = u32_at(header, 24);
        uint32_t count = u32_at(header, 28);
        uint8_t body[256];
        if (list_len + packet_len + count * 16 > sizeof(body)) _exit(4);
        if (read_all(fd, body, (int)(list_len + packet_len + count * 16)) != 0) _exit(5);
        uint8_t ack[16] = {'E', 'P', 'A', 'K', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0};
        put_u32(ack, 4, 2);
        put_u32(ack, 8, frame_id);
        put_u32(ack, 12, 0);
        write(fd, ack, 16);
        close(fd);
        close(server);
        unlink(path);
        _exit(hello_id == 42 && frame_id == 42 && list_len == 3 ? 0 : 6);
    }

    void* h = host_surface_socket_open((const uint8_t*)path, (int32_t)strlen(path), 42, 320, 200);
    if (!h) {
        printf("surface socket connect failed\n");
        return 1;
    }
    int32_t damage[4] = {0, 0, 320, 200};
    uint8_t list[3] = {1, 2, 3};
    long status = host_surface_socket_commit(h, list, 3, NULL, 0, damage, 1);
    printf("surface socket status=%ld id=%ld\n", status, host_surface_socket_id(h));
    host_surface_socket_close(h);
    close(server);
    int code = 0;
    waitpid(child, &code, 0);
    unlink(path);
    return (status == 0 && WIFEXITED(code) && WEXITSTATUS(code) == 0) ? 0 : 1;
}
