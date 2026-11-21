#import <Foundation/Foundation.h>

// Let Finder launch again when the operator closes the browser tab. The Rust
// process owns the listening socket and reopens the same control room on demand.
int main(void) {
    @autoreleasepool {
        NSURL *bundle = NSBundle.mainBundle.bundleURL;
        NSTask *core = [[NSTask alloc] init];
        core.executableURL = [bundle URLByAppendingPathComponent:@"Contents/Helpers/avatar3d-live"];
        NSArray<NSString *> *arguments = NSProcessInfo.processInfo.arguments;
        core.arguments = [arguments subarrayWithRange:NSMakeRange(1, arguments.count - 1)];
        NSError *error = nil;
        if (![core launchAndReturnError:&error]) {
            NSLog(@"无法启动 Avatar3D：%@", error.localizedDescription);
            return 1;
        }
    }
    return 0;
}
