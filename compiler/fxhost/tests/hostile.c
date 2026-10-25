#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

int fx_add(int a, int b) {
    return a + b;
}

int fx_try_open_etc(void) {
    int fd = open("/etc/passwd", O_RDONLY);
    if (fd < 0) {
        return -1;
    }
    close(fd);
    return 0;
}

int fx_try_connect(void) {
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) {
        return -1;
    }
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(9);
    addr.sin_addr.s_addr = htonl(0x7F000001u);
    int r = connect(s, (struct sockaddr *)&addr, sizeof(addr));
    int e = errno;
    close(s);
    if (r < 0) {
        errno = e;
        return -1;
    }
    return 0;
}
