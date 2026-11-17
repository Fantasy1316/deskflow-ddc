#include <assert.h>
#include <stdio.h>
#include "../mac-intel-ddc.h"

int main(void) {
    IntelInputProtocol standard, alternate;
    assert(intel_input_protocol("input", &standard));
    assert(intel_input_protocol("input-alt", &alternate));
    assert(!intel_input_protocol("input-anything", &alternate));
    uint8_t data[7] = {0};
    assert(intel_input_packet(standard, false, 0, data) == 5);
    assert(memcmp(data, (uint8_t[]){0x51, 0x82, 0x01, 0x60, 0xdc}, 5) == 0);
    assert(intel_input_packet(standard, true, 17, data) == 7);
    assert(memcmp(data, (uint8_t[]){0x51, 0x84, 0x03, 0x60, 0x00, 0x11, 0xc9}, 7) == 0);
    assert(intel_input_packet(alternate, true, 208, data) == 7);
    assert(memcmp(data, (uint8_t[]){0x50, 0x84, 0x03, 0xf4, 0x00, 0xd0, 0x9d}, 7) == 0);
    assert(intel_input_packet(alternate, false, 0, data) == 5);
    assert(memcmp(data, (uint8_t[]){0x50, 0x82, 0x01, 0xf4, 0x49}, 5) == 0);
    // Fixed length: a zero checksum is still transmitted.
    assert(intel_input_packet(standard, true, 216, data) == 7 && data[6] == 0);
    uint16_t value;
    assert(intel_input_value("65535", &value) && value == 65535);
    assert(intel_input_value("0", &value) && value == 0);
    assert(!intel_input_value("65536", &value));
    assert(!intel_input_value("9999999999999999999", &value));
    assert(!intel_input_value("-1", &value));
    assert(!intel_input_value("17x", &value));
    assert(!intel_input_value("", &value));
    IntelDisplayIdentity screen = {1, 2, 3, 4, true}, match = screen;
    assert(intel_display_matches(screen, match));
    match.unit++;
    assert(!intel_display_matches(screen, match));
    match.hasUnit = false;
    assert(intel_display_matches(screen, match));
    match.serial++;
    assert(!intel_display_matches(screen, match));
    int current, maximum;
    uint8_t reply[] = {0x6e, 0x88, 0x02, 0, 0xf4, 0, 0x01, 0xff, 0, 0xd0, 0};
    reply[10] = 0x50;
    for (size_t i = 0; i < 10; ++i) reply[10] ^= reply[i];
    assert(sb_parse_ddc_reply(reply, sizeof(reply), 0xf4, &current, &maximum) && current == 208 && maximum == 511);
    assert(!sb_parse_ddc_reply(reply, sizeof(reply), 0x60, &current, &maximum));
    reply[10] ^= 1;
    assert(!sb_parse_ddc_reply(reply, sizeof(reply), 0xf4, &current, &maximum));
    puts("Intel DDC packets, protocol isolation, identity and response tests passed; no native calls");
}
