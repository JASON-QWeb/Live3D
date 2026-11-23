#import "FilePicker.h"
#import <AppKit/AppKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

void AvatarConfigureFilePicker(WKWebViewConfiguration *config) {
    // WKOpenPanelParameters supplies directory/multiple flags, but omits HTML's accept list.
    NSString *source = @"document.addEventListener('click', e => {"
                        " if (e.target instanceof HTMLInputElement && e.target.type === 'file')"
                        " globalThis.avatarAccept = e.target.accept;"
                        "}, true);";
    WKUserScript *script = [[WKUserScript alloc] initWithSource:source
        injectionTime:WKUserScriptInjectionTimeAtDocumentStart forMainFrameOnly:YES
        inContentWorld:WKContentWorld.defaultClientWorld];
    [config.userContentController addUserScript:script];
}

static NSArray<UTType *> *contentTypes(NSString *accept) {
    NSMutableOrderedSet<UTType *> *types = [NSMutableOrderedSet orderedSet];
    NSDictionary *wildcards = @{@"audio/*": UTTypeAudio, @"video/*": UTTypeMovie, @"image/*": UTTypeImage};
    for (NSString *entry in [accept componentsSeparatedByString:@","]) {
        NSString *value = [entry stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
        if (!value.length) continue;
        UTType *type = wildcards[value];
        if (!type) {
            type = [value hasPrefix:@"."] ? [UTType typeWithFilenameExtension:[value substringFromIndex:1]]
                                         : [UTType typeWithMIMEType:value];
        }
        if (type) [types addObject:type];
    }
    return types.array;
}

void AvatarChooseFiles(WKWebView *view, WKOpenPanelParameters *parameters,
                       WKFrameInfo *frame, void (^complete)(NSArray<NSURL *> *)) {
    NSOpenPanel *panel = NSOpenPanel.openPanel;
    panel.canChooseDirectories = parameters.allowsDirectories;
    panel.canChooseFiles = !parameters.allowsDirectories;
    panel.allowsMultipleSelection = parameters.allowsMultipleSelection;
    [view evaluateJavaScript:@"globalThis.avatarAccept || ''" inFrame:frame
        inContentWorld:WKContentWorld.defaultClientWorld completionHandler:^(id result, NSError *error) {
            if (!view.window) { complete(nil); return; }
            if (!error && panel.canChooseFiles && [result isKindOfClass:NSString.class]) {
                NSArray *types = contentTypes(result);
                if (types.count) panel.allowedContentTypes = types;
            }
            [panel beginSheetModalForWindow:view.window completionHandler:^(NSModalResponse response) {
                complete(response == NSModalResponseOK ? panel.URLs : nil);
            }];
        }];
}
