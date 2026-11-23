#import <WebKit/WebKit.h>

void AvatarConfigureFilePicker(WKWebViewConfiguration *config);
void AvatarChooseFiles(WKWebView *view, WKOpenPanelParameters *parameters,
                       WKFrameInfo *frame, void (^complete)(NSArray<NSURL *> *));
