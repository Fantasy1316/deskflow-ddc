#import "../mac-system-events.m"
#include <assert.h>

static NSUInteger sleeps, wakes, displays, checks;
static void receive(uint32_t event) {
    if (event == 1) sleeps++;
    if (event == 2) wakes++;
    if (event == 3) displays++;
    if (event == 4) checks++;
}

int main(void) {
    @autoreleasepool {
        // Private notification center: never sleep the Mac or reconfigure a
        // display. Exercise the same blocks used by the live registration.
        NSNotificationCenter *center = [NSNotificationCenter new];
        atomic_store(&systemCallback, receive);
        NSArray *observers = observePower(center);
        [center postNotificationName:NSWorkspaceWillSleepNotification object:nil];
        [center postNotificationName:NSWorkspaceDidWakeNotification object:nil];
        [center postNotificationName:NSWorkspaceScreensDidSleepNotification object:nil];
        [center postNotificationName:NSWorkspaceScreensDidWakeNotification object:nil];
        [center postNotificationName:NSWorkspaceSessionDidBecomeActiveNotification object:nil];
        assert(sleeps == 1 && wakes == 1 && displays == 0 && checks == 2);
        displayChanged(1, kCGDisplayBeginConfigurationFlag, NULL);
        assert(displays == 0);
        displayChanged(1, kCGDisplayAddFlag, NULL);
        displayChanged(1, kCGDisplayRemoveFlag, NULL);
        assert(displays == 2);
        atomic_store(&systemCallback, NULL);
        displayChanged(1, kCGDisplaySetModeFlag, NULL);
        [center postNotificationName:NSWorkspaceDidWakeNotification object:nil];
        [center postNotificationName:NSWorkspaceScreensDidWakeNotification object:nil];
        assert(displays == 2 && wakes == 1 && checks == 2);
        for (id observer in observers) [center removeObserver:observer];
        atomic_store(&systemCallback, receive);
        [center postNotificationName:NSWorkspaceWillSleepNotification object:nil];
        [center postNotificationName:NSWorkspaceScreensDidWakeNotification object:nil];
        [center postNotificationName:NSWorkspaceSessionDidBecomeActiveNotification object:nil];
        assert(sleeps == 1 && checks == 2);
        atomic_store(&systemCallback, NULL);
    }
    return 0;
}
