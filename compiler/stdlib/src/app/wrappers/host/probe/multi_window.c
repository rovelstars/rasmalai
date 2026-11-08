#include "host_cabi.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

static void settle(host h) {
    for (int i = 0; i < 200; i++) {
        host_pump_timeout(h, 0);
        usleep(5000);
    }
}

static void put_u32(uint8_t* p, uint32_t v) { memcpy(p, &v, 4); }
static void put_f64(uint8_t* p, double v) { memcpy(p, &v, 8); }

static int build_list(uint8_t* out, int w, int h, uint32_t color) {
    memset(out, 0, 96);
    put_u32(out + 0, 0x44584E52u);
    put_u32(out + 4, 2u);
    put_u32(out + 8, 1u);
    put_u32(out + 12, 64u);
    put_u32(out + 16, 96u);
    put_u32(out + 24, (uint32_t)w);
    put_u32(out + 28, (uint32_t)h);
    put_u32(out + 32, 1u);
    put_u32(out + 36, color);
    put_f64(out + 48, 0.0);
    put_f64(out + 56, 0.0);
    put_f64(out + 64, (double)w);
    put_f64(out + 72, (double)h);
    return 96;
}

int main(void) {
    host h = host_open((const uint8_t*)"multi-a", 7, 320, 200, 0);
    if (!h) {
        printf("no host\n");
        return 1;
    }
    settle(h);
    long id = host_open_window(h, (const uint8_t*)"multi-b", 7, 280, 180);
    if (id != 1) {
        printf("second window not opened: %ld\n", id);
        host_close(h);
        return 1;
    }
    settle(h);
    printf("count=%ld opened=%ld\n", host_window_count(h), id);

    uint8_t list[96];
    int32_t damage[4] = {0, 0, 160, 120};
    int len = build_list(list, 160, 120, 0x2B2F38FFu);
    long p0 = host_present_window(h, 0, list, len, damage, 1);
    long p1 = host_present_window(h, 1, list, len, damage, 1);
    settle(h);
    printf("present primary=%ld second=%ld\n", p0, p1);
    long gen0 = host_window_generation(h, 0);
    long gen1 = host_window_generation(h, 1);
    printf("gen primary=%ld second=%ld\n", gen0, gen1);
    host_close(h);
    return (p0 <= 1 && p1 <= 1 && gen0 > 0 && gen1 > 0) ? 0 : 1;
}
