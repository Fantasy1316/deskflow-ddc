#import <AppKit/AppKit.h>
#import <objc/runtime.h>

static void (*requestQuit)(void);
static NSApplication *pendingApplication;

static NSApplicationTerminateReply shouldTerminate(id self, SEL selector, NSApplication *app) {
    (void)self;
    (void)selector;
    // Preserve Cocoa's termination transaction, including logout/shutdown.
    // Reply only after Rust has finished reaping the owned engine.
    if (!pendingApplication) {
        pendingApplication = app;
        // The reply must occur after applicationShouldTerminate: returns Later.
        dispatch_async(dispatch_get_main_queue(), ^{ requestQuit(); });
    }
    return NSTerminateLater;
}

// Called on the main thread. False means this exit was initiated through
// Tauri rather than an outstanding Cocoa termination request.
bool sb_complete_quit(bool allow) {
    NSApplication *app = pendingApplication;
    if (!app) return false;
    pendingApplication = nil;
    [app replyToApplicationShouldTerminate:allow];
    return true;
}

bool sb_install_quit_handler(void (*callback)(void)) {
    if (!callback || !NSApp.delegate) return false;
    requestQuit = callback;
    id<NSApplicationDelegate> delegate = NSApp.delegate;
    if (!class_addMethod(object_getClass(delegate),
        @selector(applicationShouldTerminate:), (IMP)shouldTerminate, "Q@:@")) return false;
    // AppKit may cache which optional delegate methods exist in setDelegate:.
    NSApp.delegate = nil;
    NSApp.delegate = delegate;
    return true;
}
