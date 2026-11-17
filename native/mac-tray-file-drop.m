#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>

typedef bool (*DDCFileDropAction)(int32_t kind, const char *paths);

// Keep NSStatusBarButton and its existing target/action/menu intact. The
// transparent child handles only drag sessions and forwards ordinary clicks.
@interface DDCTrayFileDropView : NSView <NSDraggingDestination>
@property(nonatomic) DDCFileDropAction action;
@property(nonatomic) BOOL dragging;
@property(nonatomic, weak) NSView *clickTarget;
@end

@implementation DDCTrayFileDropView
- (BOOL)isAccessibilityElement { return NO; }
- (BOOL)acceptsFirstMouse:(NSEvent *)event { (void)event; return YES; }
- (void)mouseDown:(NSEvent *)event { [self.clickTarget mouseDown:event]; }
- (void)mouseUp:(NSEvent *)event { [self.clickTarget mouseUp:event]; }
- (void)rightMouseDown:(NSEvent *)event { [self.clickTarget rightMouseDown:event]; }
- (void)rightMouseUp:(NSEvent *)event { [self.clickTarget rightMouseUp:event]; }
- (void)otherMouseDown:(NSEvent *)event { [self.clickTarget otherMouseDown:event]; }
- (void)otherMouseUp:(NSEvent *)event { [self.clickTarget otherMouseUp:event]; }
- (void)mouseDragged:(NSEvent *)event { [self.clickTarget mouseDragged:event]; }
- (BOOL)hasFiles:(id<NSDraggingInfo>)sender {
    return [sender.draggingPasteboard canReadObjectForClasses:@[NSURL.class]
        options:@{NSPasteboardURLReadingFileURLsOnlyKey:@YES}];
}
- (NSDragOperation)draggingEntered:(id<NSDraggingInfo>)sender {
    self.dragging = [self hasFiles:sender] && (sender.draggingSourceOperationMask & NSDragOperationCopy)
        && self.action && self.action(1, NULL);
    self.needsDisplay = YES;
    return self.dragging && (sender.draggingSourceOperationMask & NSDragOperationCopy) ? NSDragOperationCopy : NSDragOperationNone;
}
- (NSDragOperation)draggingUpdated:(id<NSDraggingInfo>)sender {
    return self.dragging && (sender.draggingSourceOperationMask & NSDragOperationCopy) ? NSDragOperationCopy : NSDragOperationNone;
}
- (void)draggingExited:(id<NSDraggingInfo>)sender {
    (void)sender; self.dragging = NO; self.needsDisplay = YES;
    if (self.action) self.action(4, NULL);
}
- (BOOL)prepareForDragOperation:(id<NSDraggingInfo>)sender {
    return self.dragging && [self hasFiles:sender]
        && (sender.draggingSourceOperationMask & NSDragOperationCopy);
}
- (BOOL)performDragOperation:(id<NSDraggingInfo>)sender {
    if (![self prepareForDragOperation:sender] || !self.action) return NO;
    self.dragging = NO;
    self.needsDisplay = YES;
    NSArray<NSURL *> *urls = [sender.draggingPasteboard readObjectsForClasses:@[NSURL.class]
        options:@{NSPasteboardURLReadingFileURLsOnlyKey:@YES}];
    if (urls.count == 0 || urls.count > 256) return NO;
    NSMutableArray<NSString *> *paths = [NSMutableArray arrayWithCapacity:urls.count];
    for (NSURL *url in urls) {
        if (!url.isFileURL || !url.path.isAbsolutePath) return NO;
        [paths addObject:url.path];
    }
    NSData *data = [NSJSONSerialization dataWithJSONObject:paths options:0 error:nil];
    NSString *json = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;
    return json && self.action(2, json.UTF8String);
}
- (void)concludeDragOperation:(id<NSDraggingInfo>)sender {
    (void)sender; self.dragging = NO; self.needsDisplay = YES;
    if (self.action) self.action(4, NULL);
}
- (void)draggingEnded:(id<NSDraggingInfo>)sender {
    (void)sender; self.dragging = NO; self.needsDisplay = YES;
    if (self.action) self.action(4, NULL);
}
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    if (self.dragging) {
        [[NSColor.controlAccentColor colorWithAlphaComponent:0.22] setFill];
        [[NSBezierPath bezierPathWithRoundedRect:NSInsetRect(self.bounds, 1, 1) xRadius:5 yRadius:5] fill];
    }
}
@end

bool ddc_file_drop_install(void *statusItem, DDCFileDropAction callback) {
    if (!NSThread.isMainThread || !statusItem || !callback) return false;
    NSStatusBarButton *button = ((__bridge NSStatusItem *)statusItem).button;
    if (!button) return false;
    DDCTrayFileDropView *drop = nil;
    for (NSView *child in button.subviews) {
        if ([child isKindOfClass:DDCTrayFileDropView.class]) { drop = (DDCTrayFileDropView *)child; break; }
    }
    if (!drop) {
        // tray-icon installs its mouse handler in a child of the status button.
        // Capture the original hit-test recipient before covering it, rather
        // than forwarding to NSStatusBarButton and bypassing that handler.
        NSPoint center = NSMakePoint(NSMidX(button.bounds), NSMidY(button.bounds));
        NSView *target = [button hitTest:[button convertPoint:center toView:button.superview]];
        drop = [[DDCTrayFileDropView alloc] initWithFrame:button.bounds];
        drop.clickTarget = target ?: button;
        drop.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [drop registerForDraggedTypes:@[NSPasteboardTypeFileURL]];
        [button addSubview:drop];
    }
    drop.action = callback;
    return true;
}

// The file panel closes on outside clicks and app/window deactivation. Unlike
// the source-confirmation popover this also covers hover-opened, non-key windows.
@interface DDCFilePanelObserver : NSObject
@property(nonatomic, weak) NSWindow *window;
@property(nonatomic) DDCFileDropAction action;
@property(nonatomic) id localMonitor;
@property(nonatomic) id globalMonitor;
@property(nonatomic) id keyObserver;
@property(nonatomic) id appObserver;
- (void)stop;
- (void)dismiss;
@end
@implementation DDCFilePanelObserver
- (void)dismiss {
    if (self.window.visible && self.action) self.action(3, NULL);
}
- (void)stop {
    if (self.localMonitor) [NSEvent removeMonitor:self.localMonitor];
    if (self.globalMonitor) [NSEvent removeMonitor:self.globalMonitor];
    if (self.keyObserver) [NSNotificationCenter.defaultCenter removeObserver:self.keyObserver];
    if (self.appObserver) [NSNotificationCenter.defaultCenter removeObserver:self.appObserver];
    self.localMonitor = self.globalMonitor = self.keyObserver = self.appObserver = nil;
}
@end
static DDCFilePanelObserver *filePanelObserver;

void ddc_file_drop_hide(void) {
    [filePanelObserver stop];
}

static void observePanel(NSWindow *window, DDCFileDropAction callback) {
    if (!filePanelObserver) filePanelObserver = [DDCFilePanelObserver new];
    [filePanelObserver stop];
    filePanelObserver.window = window;
    filePanelObserver.action = callback;
    __weak DDCFilePanelObserver *observer = filePanelObserver;
    NSEventMask clicks = NSEventMaskLeftMouseDown | NSEventMaskRightMouseDown | NSEventMaskOtherMouseDown;
    filePanelObserver.localMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:clicks handler:^NSEvent *(NSEvent *event) {
        if (event.window != observer.window && event.window.sheetParent != observer.window) [observer dismiss];
        return event;
    }];
    filePanelObserver.globalMonitor = [NSEvent addGlobalMonitorForEventsMatchingMask:clicks handler:^(NSEvent *event) {
        (void)event;
        [observer dismiss];
    }];
    filePanelObserver.keyObserver = [NSNotificationCenter.defaultCenter addObserverForName:NSWindowDidResignKeyNotification
        object:window queue:nil usingBlock:^(NSNotification *note) { (void)note; [observer dismiss]; }];
    filePanelObserver.appObserver = [NSNotificationCenter.defaultCenter addObserverForName:NSApplicationDidResignActiveNotification
        object:NSApp queue:nil usingBlock:^(NSNotification *note) { (void)note; [observer dismiss]; }];
}

// Clip the entire native content layer, including the WebView's opaque backing.
// CSS border-radius alone would leave rectangular WKWebView corners visible.
static CGPathRef panelShape(CGFloat width, CGFloat height, CGFloat arrowX) CF_RETURNS_RETAINED;
static CGPathRef panelShape(CGFloat width, CGFloat height, CGFloat arrowX) {
    CGMutablePathRef path = CGPathCreateMutable();
    CGPathAddRoundedRect(path, NULL, CGRectMake(0, 0, width, height - 10), 14, 14);
    CGPathMoveToPoint(path, NULL, arrowX - 10, height - 10);
    CGPathAddLineToPoint(path, NULL, arrowX, height);
    CGPathAddLineToPoint(path, NULL, arrowX + 10, height - 10);
    CGPathCloseSubpath(path);
    return path;
}

bool ddc_file_drop_place(void *statusItem, void *panel, double width, double height, DDCFileDropAction callback) {
    if (!NSThread.isMainThread || !statusItem || !panel) return false;
    NSStatusBarButton *button = ((__bridge NSStatusItem *)statusItem).button;
    NSWindow *window = (__bridge NSWindow *)panel;
    NSScreen *screen = button.window.screen;
    if (!button.window || !screen) return false;
    NSRect anchor = [button.window convertRectToScreen:[button convertRect:button.bounds toView:nil]];
    NSRect work = screen.visibleFrame;
    NSRect frame = NSMakeRect(NSMidX(anchor) - width / 2, NSMinY(anchor) - height - 2, width, height);
    frame.origin.x = fmax(NSMinX(work), fmin(frame.origin.x, NSMaxX(work) - width));
    frame.origin.y = fmax(NSMinY(work), fmin(frame.origin.y, NSMaxY(work) - height));
    [window setFrame:frame display:NO];
    window.opaque = NO;
    window.backgroundColor = NSColor.clearColor;
    window.hasShadow = YES;
    NSView *content = window.contentView;
    content.wantsLayer = YES;
    CGFloat arrowX = fmax(24, fmin(NSMidX(anchor) - frame.origin.x, width - 24));
    CAShapeLayer *mask = [CAShapeLayer layer];
    mask.frame = content.bounds;
    // AppKit's layer follows the content view's coordinate orientation.
    CGPathRef path = panelShape(width, height, arrowX);
    if (content.isFlipped) {
        CGAffineTransform flip = CGAffineTransformMake(1, 0, 0, -1, 0, height);
        CGPathRef flipped = CGPathCreateCopyByTransformingPath(path, &flip);
        CGPathRelease(path);
        path = flipped;
    }
    mask.path = path;
    CGPathRelease(path);
    content.layer.mask = mask;
    [window invalidateShadow];
    observePanel(window, callback);
    return true;
}
