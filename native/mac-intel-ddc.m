// MIT. IOKit transport informed by MonitorControl's IntelDDC (see bundled notices).
#import <AppKit/AppKit.h>
#import <IOKit/graphics/IOGraphicsLib.h>
#import <IOKit/i2c/IOI2CInterface.h>
#include <dlfcn.h>
#include <mach/mach_time.h>
#include <unistd.h>
#include "mac-intel-ddc.h"

static NSString *displayUUID(CGDirectDisplayID display) {
    CFUUIDRef uuid = CGDisplayCreateUUIDFromDisplayID(display);
    if (!uuid) return nil;
    NSString *text = CFBridgingRelease(CFUUIDCreateString(NULL, uuid));
    CFRelease(uuid);
    return text;
}

static NSArray<NSNumber *> *externalDisplays(void) {
    CGDirectDisplayID displays[64];
    uint32_t count = 0;
    if (CGGetOnlineDisplayList(64, displays, &count) != kCGErrorSuccess || count >= 64)
        [NSException raise:@"IntelDDC" format:@"无法枚举外接显示器，请稍后重试"];
    NSMutableArray *result = [NSMutableArray array];
    for (uint32_t i = 0; i < count; ++i)
        if (!CGDisplayIsBuiltin(displays[i]) && displayUUID(displays[i])) [result addObject:@(displays[i])];
    return result;
}

static IntelDisplayIdentity displayIdentity(CGDirectDisplayID display) {
    return (IntelDisplayIdentity){CGDisplayVendorNumber(display), CGDisplayModelNumber(display),
                                 CGDisplaySerialNumber(display), CGDisplayUnitNumber(display), true};
}

static NSDictionary *framebufferInfo(io_service_t service) {
    return CFBridgingRelease(IODisplayCreateInfoDictionary(service, kIODisplayOnlyPreferredName));
}

static bool framebufferMatches(CGDirectDisplayID display, io_service_t service) {
    NSDictionary *info = framebufferInfo(service);
    if (!info[@kDisplayVendorID] || !info[@kDisplayProductID]) return false;
    IntelDisplayIdentity candidate = {
        [info[@kDisplayVendorID] unsignedIntValue], [info[@kDisplayProductID] unsignedIntValue],
        [info[@kDisplaySerialNumber] unsignedIntValue], 0, false
    };
    NSString *location = info[@kIODisplayLocationKey];
    if ([location isKindOfClass:NSString.class]) {
        NSRegularExpression *regex = [NSRegularExpression regularExpressionWithPattern:@"@([0-9]+)[^@]+$" options:0 error:NULL];
        NSTextCheckingResult *match = [regex firstMatchInString:location options:0 range:NSMakeRange(0, location.length)];
        if (match) { candidate.hasUnit = true; candidate.unit = [[location substringWithRange:[match rangeAtIndex:1]] intValue]; }
    }
    return intel_display_matches(displayIdentity(display), candidate);
}

// Returned ports are owned. Resolve again for every invocation: framebuffer and
// CG display IDs may change after sleep, hotplug or a topology rebuild.
static io_service_t framebuffer(CGDirectDisplayID display) {
    typedef void (*ServiceForDisplay)(CGDirectDisplayID, io_service_t *);
    ServiceForDisplay lookup = (ServiceForDisplay)dlsym(RTLD_DEFAULT, "CGSServiceForDisplayNumber");
    io_service_t direct = IO_OBJECT_NULL;
    if (lookup) lookup(display, &direct);
    if (direct) {
        IOItemCount buses = 0;
        if (framebufferMatches(display, direct) && IOFBGetI2CInterfaceCount(direct, &buses) == KERN_SUCCESS && buses) return direct;
        IOObjectRelease(direct);
    }
    io_iterator_t iterator = IO_OBJECT_NULL;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching(IOFRAMEBUFFER_CONFORMSTO), &iterator) != KERN_SUCCESS)
        return IO_OBJECT_NULL;
    io_service_t matched = IO_OBJECT_NULL, service;
    bool ambiguous = false;
    while ((service = IOIteratorNext(iterator))) {
        IOItemCount buses = 0;
        if (framebufferMatches(display, service) && IOFBGetI2CInterfaceCount(service, &buses) == KERN_SUCCESS && buses) {
            if (matched) ambiguous = true;
            else { IOObjectRetain(service); matched = service; }
        }
        IOObjectRelease(service);
    }
    IOObjectRelease(iterator);
    if (ambiguous && matched) { IOObjectRelease(matched); return IO_OBJECT_NULL; }
    return matched;
}

static uint64_t transactionTypes(io_service_t interface) {
    id property = CFBridgingRelease(IORegistryEntryCreateCFProperty(interface, CFSTR(kIOI2CTransactionTypesKey), NULL, 0));
    return [property isKindOfClass:NSNumber.class] ? [property unsignedLongLongValue] : 0;
}

static uint64_t replyDelay(void) {
    mach_timebase_info_data_t timebase;
    mach_timebase_info(&timebase);
    return (40000000ull * timebase.denom) / timebase.numer;
}

static IOReturn sendInput(IOI2CConnectRef connection, IntelInputProtocol protocol, bool write,
                         uint16_t value, IOOptionBits replyType, int *current) {
    uint8_t data[7] = {0}, reply[11] = {0};
    IOI2CRequest request = {0};
    request.sendAddress = 0x6e;
    request.sendTransactionType = kIOI2CSimpleTransactionType;
    request.sendBuffer = (vm_address_t)data;
    request.sendBytes = (uint32_t)intel_input_packet(protocol, write, value, data);
    request.replyTransactionType = write ? kIOI2CNoTransactionType : replyType;
    if (!write) {
        request.replyAddress = 0x6f;
        request.replySubAddress = protocol.source;
        request.replyBuffer = (vm_address_t)reply;
        request.replyBytes = sizeof(reply);
        request.minReplyDelay = replyDelay();
    }
    usleep(10000);
    IOReturn status = IOI2CSendRequest(connection, 0, &request);
    if (status == KERN_SUCCESS) status = request.result;
    if (status != KERN_SUCCESS || write) return status;
    int maximum;
    return sb_parse_ddc_reply(reply, request.replyBytes, protocol.feature, current, &maximum)
        ? KERN_SUCCESS : kIOReturnBadMessageID;
}

static bool controlInput(io_service_t service, IntelInputProtocol protocol, bool write, uint16_t value,
                         int *current, NSString **detail) {
    IOItemCount count = 0;
    IOReturn status = IOFBGetI2CInterfaceCount(service, &count);
    if (status != KERN_SUCCESS || !count || count > 64) {
        *detail = [NSString stringWithFormat:@"显示连接未提供可用的 I²C 通道（0x%08x）", status];
        return false;
    }
    const double deadline = NSProcessInfo.processInfo.systemUptime + 1.8;
    for (IOOptionBits bus = 0; bus < count && NSProcessInfo.processInfo.systemUptime < deadline; ++bus) {
        io_service_t interface = IO_OBJECT_NULL;
        status = IOFBCopyI2CInterfaceForBus(service, bus, &interface);
        if (status != KERN_SUCCESS) continue;
        uint64_t types = transactionTypes(interface);
        IOI2CConnectRef connection = NULL;
        status = IOI2CInterfaceOpen(interface, 0, &connection);
        IOObjectRelease(interface);
        if (status != KERN_SUCCESS) continue;
        bool success = false;
        if (write) {
            // Do not retry a successful input write: switching can remove this bus.
            status = sendInput(connection, protocol, true, value, kIOI2CNoTransactionType, current);
            success = status == KERN_SUCCESS;
        } else {
            const IOOptionBits replies[] = { kIOI2CDDCciReplyTransactionType, kIOI2CSimpleTransactionType };
            status = kIOReturnUnsupportedMode;
            for (size_t i = 0; i < 2 && NSProcessInfo.processInfo.systemUptime < deadline; ++i) {
                // Some drivers omit the capability property. Try the two known
                // reply modes, but never probe another protocol or framebuffer.
                if (types && !(types & (1ull << replies[i]))) continue;
                status = sendInput(connection, protocol, false, 0, replies[i], current);
                if (status == KERN_SUCCESS) { success = true; break; }
            }
        }
        IOI2CInterfaceClose(connection, 0);
        if (success) return true;
    }
    *detail = [NSString stringWithFormat:@"Intel DDC %@失败（0x%08x）；请检查 DDC/CI、视频接口或转接设备。API 接受写入不代表实际画面已切换", write ? @"写入" : @"读取", status];
    return false;
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        @try {
            bool list = argc == 3 && strcmp(argv[1], "display") == 0 && strcmp(argv[2], "list") == 0;
            IntelInputProtocol protocol;
            uint16_t value = 0;
            bool write = argc == 6 && strcmp(argv[3], "set") == 0;
            if (!list && !(argc >= 5 && strcmp(argv[1], "display") == 0 &&
                ((argc == 5 && strcmp(argv[3], "get") == 0) || write) &&
                intel_input_protocol(argv[4], &protocol) && (!write || intel_input_value(argv[5], &value)))) {
                fprintf(stderr, "Usage: intel-ddc display list | display <UUID> get input|input-alt | display <UUID> set input|input-alt <0..65535>\n");
                return 2;
            }
            NSArray<NSNumber *> *displays = externalDisplays();
            if (list) {
                NSUInteger index = 0;
                for (NSNumber *number in displays) {
                    CGDirectDisplayID display = number.unsignedIntValue;
                    NSString *uuid = displayUUID(display);
                    if (!uuid) continue; // The display may disappear during enumeration.
                    NSString *name = nil;
                    for (NSScreen *screen in NSScreen.screens)
                        if ([screen.deviceDescription[@"NSScreenNumber"] unsignedIntValue] == display) { name = screen.localizedName; break; }
                    name = [[name ?: @"External display" componentsSeparatedByCharactersInSet:NSCharacterSet.newlineCharacterSet] componentsJoinedByString:@" "];
                    printf("[%lu] %s (%s)\n", (unsigned long)++index, name.UTF8String, uuid.UTF8String);
                }
                return 0;
            }
            NSString *selected = [[NSUUID alloc] initWithUUIDString:@(argv[2])].UUIDString;
            CGDirectDisplayID display = 0;
            for (NSNumber *number in displays) {
                NSString *candidate = displayUUID(number.unsignedIntValue);
                if (selected && candidate && [candidate caseInsensitiveCompare:selected] == NSOrderedSame) display = number.unsignedIntValue;
            }
            if (!display) { fprintf(stderr, "选中的外接显示器当前未连接，请重新扫描；不会使用旧屏幕编号\n"); return 1; }
            io_service_t service = framebuffer(display);
            if (!service) { fprintf(stderr, "无法唯一匹配 Intel 显示器的 I²C 通道；该接口可能不支持 DDC，或存在无法区分的同型号显示器\n"); return 1; }
            int current = 0;
            NSString *detail = nil;
            bool success = controlInput(service, protocol, write, value, &current, &detail);
            IOObjectRelease(service);
            if (!success) { fprintf(stderr, "%s\n", detail.UTF8String); return 1; }
            if (!write) printf("%d\n", current);
            return 0;
        } @catch (NSException *exception) {
            fprintf(stderr, "Intel DDC 失败：%s\n", exception.reason.UTF8String);
            return 1;
        }
    }
}
