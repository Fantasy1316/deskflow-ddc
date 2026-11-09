// Isolated UI regression test; never starts sharing or controls display outputs.
#import "../mac-source-popover.m"
#include <assert.h>
#include <stdio.h>

static int lastChoice = -1;
static uint64_t lastTicket = 0;
static NSStatusItem *testItem;

static void selected(uint64_t ticket, int choice) {
    lastTicket = ticket;
    lastChoice = choice;
}

static void verifyPopover(void) {
    assert(sourceController.popover.behavior == NSPopoverBehaviorApplicationDefined);
    [sourceController.view layoutSubtreeIfNeeded];
    assert(fabs(sourceController.view.bounds.size.width - 344) < 1);
    NSWindow *popup = sourceController.view.window;
    NSRect anchor = [testItem.button.window convertRectToScreen:
        [testItem.button convertRect:testItem.button.bounds toView:nil]];
    NSRect frame = popup.frame;
    assert(NSIntersectsRect(frame, popup.screen.visibleFrame));
    assert(NSMinX(frame) <= NSMidX(anchor) && NSMaxX(frame) >= NSMidX(anchor));
    assert(fabs(NSMaxY(frame) - NSMinY(anchor)) < 32);
    for (NSView *view in sourceController.view.subviews) {
        assert(!view.hasAmbiguousLayout);
    }

    assert(ddc_source_popover_show((__bridge void *)testItem, 7,
        "MacBook Pro — 名称非常长的主控工作电脑", "DESKTOP-WINDOWS-名称非常长的接收电脑",
        true, "模拟同步失败，请再次确认当前画面。", selected));
    assert(!sourceController.localButton.enabled && !sourceController.peerButton.enabled);
    assert(sourceController.localButton.toolTip.length > 20);
    assert([sourceController.statusLabel.stringValue containsString:@"模拟同步失败"]);
    ddc_source_popover_hide(6);
    assert(sourceController.popover.shown);
    NSWindow *other = [[NSWindow alloc] initWithContentRect:NSMakeRect(40,40,100,80)
        styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    [other makeKeyAndOrderFront:nil];
    assert(sourceController.popover.shown);
    [other orderOut:nil];
    [sourceController local:nil]; assert(lastTicket == 7 && lastChoice == 1);
    [sourceController peer:nil]; assert(lastTicket == 7 && lastChoice == 2);
    [sourceController dismiss:nil]; assert(lastTicket == 7 && lastChoice == 0);
    ddc_source_popover_hide(7);
    assert(!sourceController.popover.shown);
    assert(ddc_source_popover_show((__bridge void *)testItem, 8,
        "MacBook Pro", "Windows PC", false, NULL, selected));
    ddc_source_popover_hide(7);
    assert(sourceController.pending);
    ddc_source_popover_hide(8);
    assert(!sourceController.pending);
    [NSStatusBar.systemStatusBar removeStatusItem:testItem];
    puts("Native popover anchoring, layout, focus persistence and ticket checks passed.");
    exit(0);
}

int main(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        [NSApp finishLaunching];
        testItem = [NSStatusBar.systemStatusBar statusItemWithLength:NSVariableStatusItemLength];
        testItem.button.title = @"DDC Test";
        dispatch_async(dispatch_get_main_queue(), ^{
            assert(ddc_source_popover_show((__bridge void *)testItem, 7,
                "MacBook Pro", "Windows PC", false, NULL, selected));
            // AppKit positions a new popover on the next event turn.
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 300 * NSEC_PER_MSEC),
                dispatch_get_main_queue(), ^{ verifyPopover(); });
        });
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 8 * NSEC_PER_SEC),
            dispatch_get_main_queue(), ^{ fputs("Native popover test timed out.\n", stderr); exit(2); });
        [NSApp run];
    }
    return 0;
}
