#import "../mac-lifecycle.m"
#include <assert.h>

// No NSApplication/GUI is created: exercise the actual termination delegate
// protocol with a probe that records Cocoa's completion reply.
@interface TerminationProbe : NSObject
@property NSUInteger replies;
@property BOOL allowed;
@end
@implementation TerminationProbe
- (void)replyToApplicationShouldTerminate:(BOOL)allow {
    self.replies++;
    self.allowed = allow;
}
@end

static NSUInteger requests;
static void request(void) { requests++; }

int main(void) {
    @autoreleasepool {
        requestQuit = request;
        for (NSUInteger attempt = 0; attempt < 2; attempt++) {
            TerminationProbe *probe = [TerminationProbe new];
            NSApplication *app = (NSApplication *)probe;
            assert(shouldTerminate(nil, NULL, app) == NSTerminateLater);
            assert(shouldTerminate(nil, NULL, app) == NSTerminateLater);
            assert(requests == attempt); // callback cannot reply before Later
            assert(probe.replies == 0);
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:2];
            while (requests == attempt && deadline.timeIntervalSinceNow > 0) {
                [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
            }
            assert(requests == attempt + 1); // repeated Quit starts one cleanup
            assert(probe.replies == 0); // termination still pending during cleanup
            bool allow = attempt == 0;
            assert(sb_complete_quit(allow));
            assert(probe.replies == 1 && probe.allowed == allow);
            assert(!sb_complete_quit(allow)); // exactly one Cocoa reply
            assert(probe.replies == 1);
        }
    }
    return 0;
}
