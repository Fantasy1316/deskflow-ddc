#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>
#include <stdatomic.h>

typedef void (*SBSystemEvent)(uint32_t);
static _Atomic(SBSystemEvent) systemCallback;
static NSArray *powerObservers;

static void displayChanged(CGDirectDisplayID display, CGDisplayChangeSummaryFlags flags, void *context) {
    (void)display;
    (void)context;
    // A transaction sends BeginConfiguration before the new layout is ready.
    if (flags & kCGDisplayBeginConfigurationFlag) return;
    SBSystemEvent callback = atomic_load(&systemCallback);
    if (callback) callback(3);
}

static NSArray *observePower(NSNotificationCenter *center) {
    id sleep = [center addObserverForName:NSWorkspaceWillSleepNotification object:nil queue:nil usingBlock:^(NSNotification *note) {
        (void)note;
        SBSystemEvent callback = atomic_load(&systemCallback);
        if (callback) callback(1);
    }];
    id wake = [center addObserverForName:NSWorkspaceDidWakeNotification object:nil queue:nil usingBlock:^(NSNotification *note) {
        (void)note;
        SBSystemEvent callback = atomic_load(&systemCallback);
        if (callback) callback(2);
    }];
    id screenWake = [center addObserverForName:NSWorkspaceScreensDidWakeNotification object:nil queue:nil usingBlock:^(NSNotification *note) {
        (void)note;
        SBSystemEvent callback = atomic_load(&systemCallback);
        if (callback) callback(4);
    }];
    id sessionActive = [center addObserverForName:NSWorkspaceSessionDidBecomeActiveNotification object:nil queue:nil usingBlock:^(NSNotification *note) {
        (void)note;
        SBSystemEvent callback = atomic_load(&systemCallback);
        if (callback) callback(4);
    }];
    // Display sleep is deliberately NOT system sleep: our signal fallback can
    // itself put a display to sleep and must never trigger a reconnect loop.
    // Screen/session wake only requests a check; it never resets a live session.
    return @[sleep, wake, screenWake, sessionActive];
}

bool sb_install_system_events(SBSystemEvent callback) {
    if (!callback || powerObservers) return false;
    atomic_store(&systemCallback, callback);
    if (CGDisplayRegisterReconfigurationCallback(displayChanged, NULL) != kCGErrorSuccess) {
        atomic_store(&systemCallback, NULL);
        return false;
    }
    powerObservers = observePower(NSWorkspace.sharedWorkspace.notificationCenter);
    return true;
}

void sb_remove_system_events(void) {
    atomic_store(&systemCallback, NULL);
    if (!powerObservers) return;
    NSNotificationCenter *center = NSWorkspace.sharedWorkspace.notificationCenter;
    for (id token in powerObservers) [center removeObserver:token];
    powerObservers = nil;
    CGDisplayRemoveReconfigurationCallback(displayChanged, NULL);
}
