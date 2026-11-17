// Exercise real request construction and bus cleanup against simulated IOKit.
// This test never enumerates a display or accesses an actual I2C interface.
#import <AppKit/AppKit.h>
#import <IOKit/graphics/IOGraphicsLib.h>
#import <IOKit/i2c/IOI2CInterface.h>
#include <assert.h>

static unsigned opens, closes, releases, requests;
static bool corruptReply, rejectRequest;
static IOI2CRequest lastRequest;
static uint8_t lastPacket[7];
static IOReturn fakeCount(io_service_t port, IOItemCount *count) { assert(port == 10); *count = 2; return 0; }
static IOReturn fakeBus(io_service_t port, IOOptionBits bus, io_service_t *interface) { assert(port == 10); *interface = 20 + bus; return 0; }
static CFTypeRef fakeProperty(io_registry_entry_t port, CFStringRef name, CFAllocatorRef allocator, IOOptionBits options) {
    (void)port; (void)name; (void)allocator; (void)options;
    int64_t types = (1 << kIOI2CSimpleTransactionType) | (1 << kIOI2CDDCciReplyTransactionType);
    return CFNumberCreate(NULL, kCFNumberSInt64Type, &types);
}
static IOReturn fakeOpen(io_service_t port, IOOptionBits options, IOI2CConnectRef *connection) {
    (void)options; opens++;
    if (port == 20) return kIOReturnNotOpen;
    *connection = (IOI2CConnectRef)(uintptr_t)port;
    return 0;
}
static IOReturn fakeRelease(io_object_t port) { assert(port == 20 || port == 21); releases++; return 0; }
static IOReturn fakeClose(IOI2CConnectRef connection, IOOptionBits options) { (void)connection; (void)options; closes++; return 0; }
static IOReturn fakeSend(IOI2CConnectRef connection, IOOptionBits options, IOI2CRequest *request) {
    (void)connection; (void)options;
    requests++;
    lastRequest = *request;
    memcpy(lastPacket, (void *)request->sendBuffer, request->sendBytes);
    if (rejectRequest) { request->result = kIOReturnNotPermitted; return 0; }
    request->result = 0;
    if (request->replyBytes) {
        uint8_t *reply = (uint8_t *)request->replyBuffer;
        memcpy(reply, (uint8_t[]){0x6e, 0x88, 0x02, 0, lastPacket[3], 0, 0x01, 0xff, 0, 0xd0, 0}, 11);
        reply[10] = 0x50;
        for (size_t i = 0; i < 10; ++i) reply[10] ^= reply[i];
        // Require the transport to reject a malformed reply and try the next
        // supported reply transaction type, without changing input protocol.
        if (corruptReply || request->replyTransactionType == kIOI2CDDCciReplyTransactionType) reply[10] ^= 1;
        request->replyBytes = 11;
    }
    return 0;
}

#define IOFBGetI2CInterfaceCount fakeCount
#define IOFBCopyI2CInterfaceForBus fakeBus
#define IORegistryEntryCreateCFProperty fakeProperty
#define IOI2CInterfaceOpen fakeOpen
#define IOObjectRelease fakeRelease
#define IOI2CInterfaceClose fakeClose
#define IOI2CSendRequest fakeSend
#define main intel_ddc_cli_main
#import "../mac-intel-ddc.m"
#undef main

int main(void) {
    @autoreleasepool {
        IntelInputProtocol protocol;
        assert(intel_input_protocol("input-alt", &protocol));
        NSString *detail = nil;
        int current = 0;
        assert(controlInput(10, protocol, false, 0, &current, &detail));
        assert(current == 208 && requests == 2 && opens == 2 && closes == 1 && releases == 2);
        assert(lastRequest.sendAddress == 0x6e && lastRequest.replyAddress == 0x6f && lastRequest.replySubAddress == 0x50);
        assert(lastRequest.sendBytes == 5 && lastPacket[0] == 0x50 && lastPacket[3] == 0xf4);
        assert(lastRequest.minReplyDelay > 0);
        unsigned before = requests;
        assert(controlInput(10, protocol, true, 144, &current, &detail));
        assert(requests == before + 1 && lastRequest.replyBytes == 0 && lastRequest.sendBytes == 7);
        assert(lastPacket[4] == 0 && lastPacket[5] == 144 && closes == 2);
        rejectRequest = true;
        assert(!controlInput(10, protocol, true, 144, &current, &detail));
        assert(detail.length && closes == 3);
        rejectRequest = false;
        corruptReply = true;
        current = -1;
        assert(!controlInput(10, protocol, false, 0, &current, &detail));
        assert(current == -1 && detail.length && closes == 4);
        assert(releases == opens && opens == 8);
        puts("Intel DDC simulated IOKit: protocol, corrupt replies, request errors and bus cleanup passed");
    }
}
