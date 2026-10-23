#ifndef SCREENBRIDGE_DDC_REPLY_H
#define SCREENBRIDGE_DDC_REPLY_H
#include <stddef.h>
#include <stdint.h>
/* DDC/CI Get VCP reply, including source byte, checked before consuming any value. */
static inline int sb_parse_ddc_reply(const uint8_t *data, size_t length, uint8_t feature, int *current, int *maximum) {
    if (length < 11 || data[0] != 0x6e || data[1] != 0x88 || data[2] != 0x02 || data[3] != 0 || data[4] != feature || data[5] > 1) return 0;
    uint8_t checksum = 0x50;
    for (size_t i = 0; i < 11; ++i) checksum ^= data[i];
    if (checksum != 0) return 0;
    *maximum = ((int)data[6] << 8) | data[7];
    *current = ((int)data[8] << 8) | data[9];
    return 1;
}
#endif
