// MIT. Intel transport packets are independent of the IOAVService backend.
#ifndef DESKFLOW_INTEL_DDC_H
#define DESKFLOW_INTEL_DDC_H
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include "ddc_reply.h"

typedef struct {
    uint8_t feature;
    uint8_t source;
} IntelInputProtocol;

static inline bool intel_input_protocol(const char *name, IntelInputProtocol *protocol) {
    if (strcmp(name, "input") == 0) *protocol = (IntelInputProtocol){0x60, 0x51};
    else if (strcmp(name, "input-alt") == 0) *protocol = (IntelInputProtocol){0xf4, 0x50};
    else return false;
    return true;
}

static inline size_t intel_input_packet(IntelInputProtocol protocol, bool write, uint16_t value, uint8_t data[7]) {
    size_t size = write ? 7 : 5;
    data[0] = protocol.source;
    data[1] = write ? 0x84 : 0x82;
    data[2] = write ? 0x03 : 0x01;
    data[3] = protocol.feature;
    if (write) { data[4] = (uint8_t)(value >> 8); data[5] = (uint8_t)value; }
    uint8_t checksum = 0x6e;
    for (size_t i = 0; i < size - 1; ++i) checksum ^= data[i];
    data[size - 1] = checksum;
    return size;
}

static inline bool intel_input_value(const char *text, uint16_t *value) {
    if (!*text) return false;
    uint32_t result = 0;
    for (const unsigned char *p = (const unsigned char *)text; *p; ++p) {
        if (*p < '0' || *p > '9') return false;
        result = result * 10 + (*p - '0');
        if (result > UINT16_MAX) return false;
    }
    *value = (uint16_t)result;
    return true;
}

typedef struct { uint32_t vendor, model, serial, unit; bool hasUnit; } IntelDisplayIdentity;
static inline bool intel_display_matches(IntelDisplayIdentity display, IntelDisplayIdentity candidate) {
    return display.vendor == candidate.vendor && display.model == candidate.model &&
           display.serial == candidate.serial && (!candidate.hasUnit || display.unit == candidate.unit);
}
#endif
