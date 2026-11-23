#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>
#import "FilePicker.h"

typedef void (*QuitCallback)(const void *);
static NSString *const InterfaceLanguageKey = @"VorelaInterfaceLanguage";

@interface AvatarDesktop : NSObject <NSApplicationDelegate, NSWindowDelegate, WKUIDelegate, WKNavigationDelegate>
@property(nonatomic, strong) NSURL *controlURL;
@property(nonatomic, strong) NSWindow *controlWindow;
@property(nonatomic, strong) NSWindow *stageWindow;
@property(nonatomic) const void *quitContext;
@property(nonatomic) QuitCallback requestQuit;
@property(nonatomic) BOOL terminating;
@end

static NSMenu *submenu(NSMenu *bar, NSString *title) {
    NSMenuItem *item = [bar addItemWithTitle:title action:nil keyEquivalent:@""];
    item.submenu = [[NSMenu alloc] initWithTitle:title];
    return item.submenu;
}

static void installMenus(AvatarDesktop *desktop) {
    NSMenu *bar = [NSMenu new];
    NSMenu *app = submenu(bar, @"Vorela");
    [app addItemWithTitle:@"隐藏 Vorela" action:@selector(hide:) keyEquivalent:@"h"];
    [app addItemWithTitle:@"退出 Vorela" action:@selector(terminate:) keyEquivalent:@"q"];
    NSMenu *edit = submenu(bar, @"编辑");
    [edit addItemWithTitle:@"撤销" action:@selector(undo:) keyEquivalent:@"z"];
    NSMenuItem *redo = [edit addItemWithTitle:@"重做" action:@selector(redo:) keyEquivalent:@"z"];
    redo.keyEquivalentModifierMask = NSEventModifierFlagCommand | NSEventModifierFlagShift;
    [edit addItem:NSMenuItem.separatorItem];
    [edit addItemWithTitle:@"剪切" action:@selector(cut:) keyEquivalent:@"x"];
    [edit addItemWithTitle:@"复制" action:@selector(copy:) keyEquivalent:@"c"];
    [edit addItemWithTitle:@"粘贴" action:@selector(paste:) keyEquivalent:@"v"];
    [edit addItemWithTitle:@"全选" action:@selector(selectAll:) keyEquivalent:@"a"];
    NSMenu *windows = submenu(bar, @"窗口");
    [windows addItemWithTitle:@"关闭窗口" action:@selector(performClose:) keyEquivalent:@"w"];
    [windows addItemWithTitle:@"最小化" action:@selector(performMiniaturize:) keyEquivalent:@"m"];
    [windows addItemWithTitle:@"直播控制台" action:@selector(showControl:) keyEquivalent:@""].target = desktop;
    NSApp.windowsMenu = windows;
    NSApp.mainMenu = bar;
}

@implementation AvatarDesktop
- (NSWindow *)windowForView:(WKWebView *)view title:(NSString *)title size:(NSSize)size {
    NSRect available = NSScreen.mainScreen.visibleFrame;
    NSRect rect = NSMakeRect(0, 0, MIN(size.width, NSWidth(available)), MIN(size.height, NSHeight(available) - 32));
    NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                              NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable;
    NSWindow *window = [[NSWindow alloc] initWithContentRect:rect styleMask:style
                                                    backing:NSBackingStoreBuffered defer:NO];
    window.releasedWhenClosed = NO;
    window.title = title;
    window.delegate = self;
    window.contentMinSize = NSMakeSize(480, 360);
    window.contentView = view;
    view.UIDelegate = self;
    view.navigationDelegate = self;
    [window center];
    [window setFrameAutosaveName:title];
    return window;
}

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    installMenus(self);
    WKWebViewConfiguration *config = [WKWebViewConfiguration new];
    config.mediaTypesRequiringUserActionForPlayback = WKAudiovisualMediaTypeNone;
    AvatarConfigureFilePicker(config);
    WKWebView *view = [[WKWebView alloc] initWithFrame:NSZeroRect configuration:config];
    self.controlWindow = [self windowForView:view title:@"Vorela" size:NSMakeSize(1560, 840)];
    [view loadRequest:[NSURLRequest requestWithURL:self.controlURL]];
    [self showControl:nil];
}

- (void)showControl:(id)sender {
    if (self.controlWindow.miniaturized) [self.controlWindow deminiaturize:nil];
    [self.controlWindow makeKeyAndOrderFront:nil];
    if (@available(macOS 14.0, *)) [NSApp activate];
}

- (BOOL)applicationShouldHandleReopen:(NSApplication *)sender hasVisibleWindows:(BOOL)visible {
    [self showControl:nil];
    return NO;
}

- (BOOL)windowShouldClose:(NSWindow *)window {
    if (window != self.controlWindow) return YES;
    [window orderOut:nil];
    return NO;
}

- (BOOL)allowNavigation:(WKNavigationAction *)action fromView:(WKWebView *)view {
    NSURL *url = action.request.URL;
    BOOL local = [url.scheme isEqualToString:self.controlURL.scheme] &&
                 [url.host isEqualToString:self.controlURL.host] && [url.port isEqual:self.controlURL.port];
    if (local || [url.absoluteString isEqualToString:@"about:blank"]) return YES;
    if (view == self.controlWindow.contentView && action.sourceFrame.mainFrame &&
        action.navigationType == WKNavigationTypeLinkActivated &&
        [url.absoluteString isEqualToString:@"https://obsproject.com/download"]) {
        [NSWorkspace.sharedWorkspace openURL:url];
    }
    return NO;
}

- (WKWebView *)webView:(WKWebView *)view createWebViewWithConfiguration:(WKWebViewConfiguration *)config
    forNavigationAction:(WKNavigationAction *)action windowFeatures:(WKWindowFeatures *)features {
    if (view != self.controlWindow.contentView) return nil;
    if (action.request.URL && ![self allowNavigation:action fromView:view]) return nil;
    [self.stageWindow close];
    WKWebView *stage = [[WKWebView alloc] initWithFrame:NSZeroRect configuration:config];
    self.stageWindow = [self windowForView:stage title:@"Vorela · 独立舞台" size:NSMakeSize(720, 800)];
    [self.stageWindow makeKeyAndOrderFront:nil];
    return stage;
}

- (void)webViewDidClose:(WKWebView *)view {
    [view.window performClose:nil];
}

- (void)windowWillClose:(NSNotification *)notification {
    if (notification.object == self.stageWindow) self.stageWindow = nil;
}

- (void)webView:(WKWebView *)view decidePolicyForNavigationAction:(WKNavigationAction *)action
    decisionHandler:(void (^)(WKNavigationActionPolicy))decide {
    if (action.targetFrame && !action.targetFrame.mainFrame) {
        decide(WKNavigationActionPolicyAllow);
        return;
    }
    decide([self allowNavigation:action fromView:view] ? WKNavigationActionPolicyAllow : WKNavigationActionPolicyCancel);
}

- (void)webView:(WKWebView *)view didFinishNavigation:(WKNavigation *)navigation {
    if (view != self.controlWindow.contentView || ![view.URL.path isEqualToString:@"/"]) return;
    // Startup alerts can appear before WebKit loads the console on the next launch.
    [view evaluateJavaScript:@"localStorage.getItem('vorela.ui-lang') || 'zh'" completionHandler:^(id language, NSError *error) {
        if (!error && ([language isEqual:@"zh"] || [language isEqual:@"en"])) {
            [NSUserDefaults.standardUserDefaults setObject:language forKey:InterfaceLanguageKey];
        }
    }];
}

- (void)webView:(WKWebView *)view runOpenPanelWithParameters:(WKOpenPanelParameters *)parameters
    initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(NSArray<NSURL *> *))complete {
    AvatarChooseFiles(view, parameters, frame, complete);
}

- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender {
    if (!self.quitContext) return NSTerminateNow;
    self.terminating = YES;
    self.requestQuit(self.quitContext);
    return NSTerminateLater;
}
@end

void vorela_run_window(const char *url, const void *context, QuitCallback requestQuit) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        __attribute__((objc_precise_lifetime)) AvatarDesktop *desktop = [AvatarDesktop new];
        desktop.controlURL = [NSURL URLWithString:[NSString stringWithUTF8String:url]];
        desktop.quitContext = context;
        desktop.requestQuit = requestQuit;
        NSApp.delegate = desktop;
        [NSApp run];
        NSApp.delegate = nil;
    }
}

void vorela_service_ended(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        AvatarDesktop *desktop = (AvatarDesktop *)NSApp.delegate;
        if (desktop.terminating) {
            [NSApp replyToApplicationShouldTerminate:YES];
        } else {
            // Return to Rust so a service/startup error can be reported to the operator.
            [NSApp stop:nil];
            NSEvent *wake = [NSEvent otherEventWithType:NSEventTypeApplicationDefined location:NSZeroPoint
                modifierFlags:0 timestamp:0 windowNumber:0 context:nil subtype:0 data1:0 data2:0];
            [NSApp postEvent:wake atStart:YES];
        }
    });
}

void vorela_startup_error(const char *message, const char *englishMessage) {
    @autoreleasepool {
        if (![NSBundle.mainBundle.bundleURL.pathExtension isEqualToString:@"app"]) return;
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        if (@available(macOS 14.0, *)) [NSApp activate];
        NSAlert *alert = [NSAlert new];
        NSString *language = [NSUserDefaults.standardUserDefaults stringForKey:InterfaceLanguageKey] ?: NSLocale.preferredLanguages.firstObject;
        BOOL english = [language hasPrefix:@"en"];
        alert.messageText = english ? @"Couldn't start Vorela" : @"无法启动 Vorela";
        alert.informativeText = [NSString stringWithUTF8String:english ? englishMessage : message];
        [alert runModal];
    }
}
