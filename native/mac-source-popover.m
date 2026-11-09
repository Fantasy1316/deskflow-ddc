#import <AppKit/AppKit.h>
#include <stdint.h>

typedef void (*DDCSourceAction)(uint64_t ticket, int choice);

// Rounded AppKit bezels keep their intrinsic height even in a taller frame.
// Draw the full button bounds so vertical padding is visible and clickable.
@interface DDCSourceButtonCell : NSButtonCell
@end
@implementation DDCSourceButtonCell
- (void)drawBezelWithFrame:(NSRect)frame inView:(NSView *)view {
    (void)view;
    NSColor *color = [NSColor.labelColor colorWithAlphaComponent:self.highlighted ? 0.14 : 0.07];
    [color setFill];
    [[NSBezierPath bezierPathWithRoundedRect:NSInsetRect(frame, 0.5, 0.5) xRadius:7 yRadius:7] fill];
}
- (void)drawInteriorWithFrame:(NSRect)frame inView:(NSView *)view {
    NSRect content = self.image && self.imagePosition != NSImageOnly ? NSInsetRect(frame, 10, 0) : frame;
    [super drawInteriorWithFrame:content inView:view];
}
@end

@interface DDCSourceButton : NSButton
@end
@implementation DDCSourceButton
+ (Class)cellClass { return DDCSourceButtonCell.class; }
- (NSEdgeInsets)alignmentRectInsets { return NSEdgeInsetsMake(0, 0, 0, 0); }
- (void)resetCursorRects {
    [super resetCursorRects];
    if (self.enabled) [self addCursorRect:self.bounds cursor:NSCursor.pointingHandCursor];
}
- (void)setEnabled:(BOOL)enabled {
    [super setEnabled:enabled];
    [self.window invalidateCursorRectsForView:self];
}
@end

@interface DDCSourceController : NSViewController
@property(nonatomic) NSPopover *popover;
@property(nonatomic) uint64_t ticket;
@property(nonatomic) DDCSourceAction action;
@property(nonatomic) BOOL pending;
@property(nonatomic) NSButton *localButton;
@property(nonatomic) NSButton *peerButton;
@property(nonatomic) NSTextField *statusLabel;
@property(nonatomic) NSLayoutConstraint *statusHeight;
- (void)updateLocal:(NSString *)local peer:(NSString *)peer busy:(BOOL)busy error:(NSString *)error;
@end

static DDCSourceController *sourceController;

static NSTextField *label(NSString *text, CGFloat size, NSColor *color) {
    NSTextField *field = [NSTextField wrappingLabelWithString:text];
    field.font = [NSFont systemFontOfSize:size];
    field.textColor = color;
    field.maximumNumberOfLines = 3;
    field.selectable = NO;
    field.translatesAutoresizingMaskIntoConstraints = NO;
    return field;
}

static NSButton *choiceButton(id target, SEL action, NSString *symbol) {
    NSButton *button = [DDCSourceButton buttonWithTitle:@"" target:target action:action];
    button.bezelStyle = NSBezelStyleRegularSquare;
    button.controlSize = NSControlSizeLarge;
    button.font = [NSFont systemFontOfSize:13 weight:NSFontWeightMedium];
    button.alignment = NSTextAlignmentLeft;
    button.lineBreakMode = NSLineBreakByTruncatingMiddle;
    button.image = [NSImage imageWithSystemSymbolName:symbol accessibilityDescription:nil];
    button.imagePosition = NSImageLeft;
    button.imageHugsTitle = YES;
    button.translatesAutoresizingMaskIntoConstraints = NO;
    [button.heightAnchor constraintEqualToConstant:44].active = YES;
    return button;
}

@implementation DDCSourceController
- (void)loadView {
    NSView *content = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 344, 270)];
    NSStackView *stack = [[NSStackView alloc] init];
    stack.orientation = NSUserInterfaceLayoutOrientationVertical;
    stack.alignment = NSLayoutAttributeLeading;
    stack.spacing = 8;
    stack.translatesAutoresizingMaskIntoConstraints = NO;
    [content addSubview:stack];
    [NSLayoutConstraint activateConstraints:@[
        [content.widthAnchor constraintEqualToConstant:344],
        [stack.leadingAnchor constraintEqualToAnchor:content.leadingAnchor constant:18],
        [stack.trailingAnchor constraintEqualToAnchor:content.trailingAnchor constant:-18],
        [stack.topAnchor constraintEqualToAnchor:content.topAnchor constant:14],
        [stack.bottomAnchor constraintEqualToAnchor:content.bottomAnchor constant:-14],
    ]];
    NSTextField *brand = label(@"Deskflow DDC", 11, NSColor.secondaryLabelColor);
    brand.font = [NSFont systemFontOfSize:11 weight:NSFontWeightSemibold];
    NSButton *close = [DDCSourceButton buttonWithImage:[NSImage imageWithSystemSymbolName:@"xmark.circle.fill" accessibilityDescription:@"关闭提醒"] target:self action:@selector(dismiss:)];
    close.bordered = NO;
    close.contentTintColor = NSColor.tertiaryLabelColor;
    close.toolTip = @"关闭提醒";
    close.translatesAutoresizingMaskIntoConstraints = NO;
    [close.widthAnchor constraintEqualToConstant:22].active = YES;
    [close.heightAnchor constraintEqualToConstant:22].active = YES;
    NSView *space = [[NSView alloc] init];
    [space setContentHuggingPriority:1 forOrientation:NSLayoutConstraintOrientationHorizontal];
    NSStackView *header = [NSStackView stackViewWithViews:@[brand, space, close]];
    header.orientation = NSUserInterfaceLayoutOrientationHorizontal;
    header.alignment = NSLayoutAttributeCenterY;
    [stack addArrangedSubview:header];
    [header.widthAnchor constraintEqualToAnchor:stack.widthAnchor].active = YES;
    NSTextField *title = label(@"共用显示器正显示哪台？", 16, NSColor.labelColor);
    title.font = [NSFont systemFontOfSize:16 weight:NSFontWeightSemibold];
    [stack addArrangedSubview:title];
    NSTextField *description = label(@"请按实际画面选择，让下一次切换方向正确。", 12, NSColor.secondaryLabelColor);
    [stack addArrangedSubview:description];
    self.localButton = choiceButton(self, @selector(local:), @"laptopcomputer");
    self.peerButton = choiceButton(self, @selector(peer:), @"display");
    [stack addArrangedSubview:self.localButton];
    [stack addArrangedSubview:self.peerButton];
    self.statusLabel = label(@"仅确认来源，不切屏或转移键鼠。", 11, NSColor.secondaryLabelColor);
    [stack addArrangedSubview:self.statusLabel];
    // Ordinary guidance needs one line; only errors reserve up to three lines.
    self.statusHeight = [self.statusLabel.heightAnchor constraintEqualToConstant:16];
    self.statusHeight.active = YES;
    NSBox *line = [[NSBox alloc] init];
    line.boxType = NSBoxSeparator;
    [stack addArrangedSubview:line];
    NSTextField *hint = label(@"未确认时，下次切换会再次提醒", 10, NSColor.secondaryLabelColor);
    NSButton *later = [DDCSourceButton buttonWithTitle:@"稍后确认" target:self action:@selector(dismiss:)];
    later.bezelStyle = NSBezelStyleRegularSquare;
    later.font = [NSFont systemFontOfSize:11 weight:NSFontWeightMedium];
    [later.heightAnchor constraintEqualToConstant:30].active = YES;
    [later.widthAnchor constraintEqualToConstant:86].active = YES;
    NSStackView *footer = [NSStackView stackViewWithViews:@[hint, later]];
    footer.orientation = NSUserInterfaceLayoutOrientationHorizontal;
    footer.alignment = NSLayoutAttributeCenterY;
    footer.spacing = 8;
    [stack addArrangedSubview:footer];
    for (NSView *view in stack.arrangedSubviews) {
        [view.widthAnchor constraintEqualToAnchor:stack.widthAnchor].active = YES;
    }
    self.view = content;
}
- (void)updateLocal:(NSString *)local peer:(NSString *)peer busy:(BOOL)busy error:(NSString *)error {
    (void)self.view;
    self.localButton.title = [@"本机：" stringByAppendingString:local];
    self.peerButton.title = [@"另一台：" stringByAppendingString:peer];
    self.localButton.toolTip = local;
    self.peerButton.toolTip = peer;
    self.localButton.accessibilityLabel = [@"当前显示本机：" stringByAppendingString:local];
    self.peerButton.accessibilityLabel = [@"当前显示另一台：" stringByAppendingString:peer];
    self.localButton.enabled = !busy;
    self.peerButton.enabled = !busy;
    self.statusLabel.stringValue = error.length ? error : (busy ? @"正在同步画面来源…" : @"仅确认来源，不切屏或转移键鼠。");
    self.statusLabel.toolTip = error.length ? error : nil;
    self.statusLabel.textColor = error.length ? NSColor.systemRedColor : NSColor.secondaryLabelColor;
    self.statusHeight.constant = error.length ? 42 : 16;
}
- (void)local:(id)sender { (void)sender; if (self.action) self.action(self.ticket, 1); }
- (void)peer:(id)sender { (void)sender; if (self.action) self.action(self.ticket, 2); }
- (void)dismiss:(id)sender { (void)sender; if (self.action) self.action(self.ticket, 0); }
- (void)cancelOperation:(id)sender { [self dismiss:sender]; }
@end

bool ddc_source_popover_show(void *statusItem, uint64_t ticket, const char *local,
                             const char *peer, bool busy, const char *error,
                             DDCSourceAction action) {
    if (!NSThread.isMainThread || !statusItem || !action) return false;
    NSStatusItem *item = (__bridge NSStatusItem *)statusItem;
    NSStatusBarButton *anchor = item.button;
    if (!anchor || !anchor.window) return false;
    if (!sourceController) {
        sourceController = [[DDCSourceController alloc] init];
        sourceController.popover = [[NSPopover alloc] init];
        sourceController.popover.contentViewController = sourceController;
        sourceController.popover.behavior = NSPopoverBehaviorApplicationDefined;
        // Context changes may close and reopen immediately. Keep those updates
        // synchronous so an old closing animation cannot hide a new prompt.
        sourceController.popover.animates = NO;
    }
    BOOL existing = (sourceController.popover.shown || sourceController.pending) && sourceController.ticket == ticket;
    if (sourceController.popover.shown && !existing) [sourceController.popover close];
    sourceController.ticket = ticket;
    sourceController.action = action;
    [sourceController updateLocal:[NSString stringWithUTF8String:local] ?: @"本机"
                             peer:[NSString stringWithUTF8String:peer] ?: @"另一台"
                             busy:busy error:error ? [NSString stringWithUTF8String:error] : nil];
    [sourceController.view layoutSubtreeIfNeeded];
    NSSize size = sourceController.view.fittingSize;
    sourceController.popover.contentSize = NSMakeSize(344, size.height);
    if (!existing) {
        // Anchor directly to NSStatusBarButton in AppKit points. No display
        // coordinate conversion, monitor scale factor, or synthetic arrow.
        // A newly created status item first needs a menu-bar layout pass;
        // presenting during app setup otherwise opens the popover at (0, 0).
        sourceController.pending = YES;
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC), dispatch_get_main_queue(), ^{
            if (!sourceController.pending || sourceController.ticket != ticket) return;
            sourceController.pending = NO;
            [NSApp activateIgnoringOtherApps:YES];
            [sourceController.popover showRelativeToRect:anchor.bounds ofView:anchor preferredEdge:NSRectEdgeMinY];
            [sourceController.popover.contentViewController.view.window makeKeyWindow];
        });
    }
    return sourceController.popover.shown || sourceController.pending;
}

void ddc_source_popover_hide(uint64_t ticket) {
    if (NSThread.isMainThread && sourceController.ticket == ticket) {
        sourceController.pending = NO;
        [sourceController.popover close];
    }
}
