#import "../mac-tray-file-drop.m"
#include <assert.h>

@interface DragProbe : NSObject
@property NSPasteboard *draggingPasteboard;
@property NSDragOperation draggingSourceOperationMask;
@end
@implementation DragProbe
@end

@interface ClickProbe : NSView
@property NSUInteger leftClicks;
@property NSUInteger rightClicks;
@end
@implementation ClickProbe
- (void)mouseDown:(NSEvent *)event { (void)event; self.leftClicks++; }
- (void)rightMouseDown:(NSEvent *)event { (void)event; self.rightClicks++; }
@end

// Match tray-icon's actual structure: the click handler is a sibling below
// our drag overlay, not the NSStatusBarButton parent.
@interface StatusProbe : NSObject
@property NSView *button;
@end
@implementation StatusProbe
@end

static NSUInteger hovers, sends, ends;
static BOOL ready = YES;
static NSArray *received;
static bool callback(int32_t kind, const char *json) {
    if (kind == 1) { hovers++; return ready; }
    if (kind == 4) { ends++; return true; }
    assert(kind == 2 && json);
    sends++;
    NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
    received = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    return ready;
}

int main(void) {
    @autoreleasepool {
        // A private pasteboard and fake dragging source: no clipboard changes,
        // actual transfers, status item or input capture are involved.
        DragProbe *source = [DragProbe new];
        source.draggingPasteboard = [NSPasteboard pasteboardWithUniqueName];
        source.draggingSourceOperationMask = NSDragOperationCopy;
        id<NSDraggingInfo> drag = (id<NSDraggingInfo>)source;
        DDCTrayFileDropView *view = [[DDCTrayFileDropView alloc] initWithFrame:NSMakeRect(0, 0, 24, 24)];
        view.action = callback;
        NSArray *paths = @[@"/tmp/文件 with space.zip", @"/tmp/directory"];
        NSMutableArray *urls = [NSMutableArray array];
        for (NSString *path in paths) [urls addObject:[NSURL fileURLWithPath:path]];
        [source.draggingPasteboard writeObjects:urls];
        assert([view draggingEntered:drag] == NSDragOperationCopy);
        assert(hovers == 1 && sends == 0);
        assert([view draggingUpdated:drag] == NSDragOperationCopy && sends == 0);
        assert([view prepareForDragOperation:drag]);
        assert([view performDragOperation:drag]);
        assert(sends == 1 && [received isEqual:paths]);
        assert(![view performDragOperation:drag] && sends == 1);
        [view concludeDragOperation:drag];
        assert(!view.dragging && ends == 1);

        ready = NO;
        assert([view draggingEntered:drag] == NSDragOperationNone);
        assert(![view performDragOperation:drag] && sends == 1);
        ready = YES;
        source.draggingSourceOperationMask = NSDragOperationMove;
        assert([view draggingEntered:drag] == NSDragOperationNone);
        assert(![view prepareForDragOperation:drag]);
        source.draggingSourceOperationMask = NSDragOperationCopy;
        assert([view draggingEntered:drag] == NSDragOperationCopy);
        [view draggingExited:drag];
        assert(![view performDragOperation:drag] && sends == 1 && ends == 2);
        assert([view draggingEntered:drag] == NSDragOperationCopy);
        [view draggingEnded:drag];
        assert(!view.dragging && ends == 3 && ![view prepareForDragOperation:drag]);

        [source.draggingPasteboard clearContents];
        [source.draggingPasteboard writeObjects:@[[NSURL URLWithString:@"https://example.com/file.zip"]]];
        NSUInteger previous = hovers;
        assert([view draggingEntered:drag] == NSDragOperationNone && previous == hovers);
        assert(![view performDragOperation:drag] && sends == 1);

        NSView *parent = [[NSView alloc] initWithFrame:NSMakeRect(20, 0, 24, 24)];
        ClickProbe *click = [[ClickProbe alloc] initWithFrame:parent.bounds];
        [parent addSubview:click];
        StatusProbe *item = [StatusProbe new];
        item.button = parent;
        assert(ddc_file_drop_install((__bridge void *)item, callback));
        DDCTrayFileDropView *overlay = (DDCTrayFileDropView *)parent.subviews.lastObject;
        assert(overlay != view && overlay.clickTarget == click);
        assert(ddc_file_drop_install((__bridge void *)item, callback));
        assert(parent.subviews.count == 2 && overlay.clickTarget == click);
        NSEvent *left = [NSEvent mouseEventWithType:NSEventTypeLeftMouseDown
            location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0
            context:nil eventNumber:1 clickCount:1 pressure:0];
        NSEvent *right = [NSEvent mouseEventWithType:NSEventTypeRightMouseDown
            location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0
            context:nil eventNumber:2 clickCount:1 pressure:0];
        [overlay mouseDown:left];
        [overlay rightMouseDown:right];
        assert(click.leftClicks == 1 && click.rightClicks == 1);

        CGPathRef shape = panelShape(380, 390, 300);
        assert(!CGPathContainsPoint(shape, NULL, CGPointMake(1, 1), NO));
        assert(CGPathContainsPoint(shape, NULL, CGPointMake(190, 190), NO));
        assert(CGPathContainsPoint(shape, NULL, CGPointMake(300, 387), NO));
        assert(!CGPathContainsPoint(shape, NULL, CGPointMake(40, 387), NO));
        CGPathRelease(shape);
        [source.draggingPasteboard releaseGlobally];
    }
    return 0;
}
