// AppKit services used by the GPUI frontend. All UI calls run on the main
// thread.
#import <Carbon/Carbon.h>
#import <Cocoa/Cocoa.h>
#import <IOKit/hidsystem/IOLLEvent.h>
#import <Network/Network.h>
#import <UserNotifications/UserNotifications.h>
#include <stdatomic.h>

bool ngterm_reduce_transparency(void) {
  return NSWorkspace.sharedWorkspace
      .accessibilityDisplayShouldReduceTransparency;
}
bool ngterm_reduce_motion(void) {
  return NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion;
}
bool ngterm_minimized(void *view) {
  return [(__bridge NSView *)view window].miniaturized;
}
bool ngterm_maximized(void *view) {
  return [(__bridge NSView *)view window].zoomed;
}

static nw_path_monitor_t networkMonitor;
void ngterm_network_start(void (*report)(int)) {
  networkMonitor = nw_path_monitor_create();
  nw_path_monitor_set_update_handler(networkMonitor, ^(nw_path_t path) {
    report(nw_path_get_status(path) == nw_path_status_satisfied ? 1 : 0);
  });
  nw_path_monitor_set_queue(networkMonitor,
                            dispatch_get_global_queue(QOS_CLASS_UTILITY, 0));
  nw_path_monitor_start(networkMonitor);
  [NSWorkspace.sharedWorkspace.notificationCenter
      addObserverForName:NSWorkspaceDidWakeNotification
                  object:nil
                   queue:NSOperationQueue.mainQueue
              usingBlock:^(NSNotification *note) {
                report(2);
              }];
}
void ngterm_accessibility_start(void (*changed)(bool, bool)) {
  changed(ngterm_reduce_transparency(), ngterm_reduce_motion());
  [NSWorkspace.sharedWorkspace.notificationCenter
      addObserverForName:
          NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification
                  object:nil
                   queue:NSOperationQueue.mainQueue
              usingBlock:^(NSNotification *note) {
                changed(ngterm_reduce_transparency(), ngterm_reduce_motion());
              }];
}

// Carbon registrations use physical keys and need no input monitoring
// permission.
static EventHotKeyRef hotkey;
static EventHandlerRef hotkeyHandler;
static void (*hotkeyCallback)(void);
static OSStatus hotkeyPressed(EventHandlerCallRef next, EventRef event,
                              void *data) {
  if (hotkeyCallback)
    hotkeyCallback();
  return noErr;
}
int ngterm_hotkey(int key, unsigned mods, void (*callback)(void)) {
  if (hotkey) {
    UnregisterEventHotKey(hotkey);
    hotkey = NULL;
  }
  hotkeyCallback = callback;
  if (key < 0)
    return 0;
  if (!hotkeyHandler) {
    EventTypeSpec event = {kEventClassKeyboard, kEventHotKeyPressed};
    OSStatus status =
        InstallEventHandler(GetApplicationEventTarget(), hotkeyPressed, 1,
                            &event, NULL, &hotkeyHandler);
    if (status)
      return status;
  }
  return RegisterEventHotKey(key, mods, (EventHotKeyID){'ngtm', 1},
                             GetApplicationEventTarget(), 0, &hotkey);
}

// A stateless translation is used only to recognize a dead-key press. AppKit
// owns the composition state.
static bool ngtermDeadKey(NSEvent *event) {
  if (event.modifierFlags &
      (NSEventModifierFlagControl | NSEventModifierFlagCommand))
    return false;
  TISInputSourceRef source = TISCopyCurrentKeyboardLayoutInputSource();
  CFDataRef data =
      source
          ? TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData)
          : NULL;
  UInt32 state = 0;
  UniChar chars[8];
  UniCharCount count = 0;
  UInt32 modifiers =
      ((event.modifierFlags & NSEventModifierFlagShift) ? shiftKey : 0) |
      ((event.modifierFlags & NSEventModifierFlagOption) ? optionKey : 0) |
      ((event.modifierFlags & NSEventModifierFlagCapsLock) ? alphaLock : 0);
  if (data)
    UCKeyTranslate((const UCKeyboardLayout *)CFDataGetBytePtr(data),
                   event.keyCode, kUCKeyActionDown, (modifiers >> 8) & 0xff,
                   LMGetKbdType(), 0, &state, 8, &count, chars);
  if (source)
    CFRelease(source);
  return state != 0;
}
// Device-dependent modifier bits must never overlap our record's low-bit flags.
unsigned ngterm_key_flags(unsigned long flags, bool repeat, bool ascii,
                          bool dead) {
  unsigned packed = flags & NSEventModifierFlagDeviceIndependentFlagsMask;
  if (repeat)
    packed |= 1;
  if (ascii)
    packed |= 2;
  if (dead)
    packed |= 4;
  if (flags & NX_DEVICERALTKEYMASK)
    packed |= 8;
  if (flags & NX_DEVICERSHIFTKEYMASK)
    packed |= 16;
  if (flags & NX_DEVICERCTLKEYMASK)
    packed |= 32;
  if (flags & NX_DEVICERCMDKEYMASK)
    packed |= 64;
  return packed;
}
int ngterm_key_event(void *pointer, unsigned *mods, unsigned *plain,
                     unsigned *command, char *text, char *withoutOption,
                     unsigned capacity) {
  NSEvent *event = (__bridge NSEvent *)pointer;
  if (!event ||
      (event.type != NSEventTypeKeyDown && event.type != NSEventTypeKeyUp))
    return -1;
  TISInputSourceRef source = TISCopyCurrentKeyboardInputSource();
  CFBooleanRef ascii =
      source ? TISGetInputSourceProperty(source,
                                         kTISPropertyInputSourceIsASCIICapable)
             : NULL;
  *mods =
      ngterm_key_flags(event.modifierFlags, event.isARepeat,
                       ascii && CFBooleanGetValue(ascii), ngtermDeadKey(event));
  if (source)
    CFRelease(source);
  NSString *base = [event charactersByApplyingModifiers:0];
  NSString *cmd =
      [event charactersByApplyingModifiers:NSEventModifierFlagCommand];
  *plain = base.length ? [base characterAtIndex:0] : 0;
  *command = cmd.length ? [cmd characterAtIndex:0] : 0;
  NSEventModifierFlags held =
      event.modifierFlags &
      ~(NSEventModifierFlagControl | NSEventModifierFlagCommand);
  NSString *typed = [event charactersByApplyingModifiers:held];
  NSString *unmodified =
      [event charactersByApplyingModifiers:held & ~NSEventModifierFlagOption];
  if (![typed getCString:text maxLength:capacity encoding:NSUTF8StringEncoding])
    text[0] = 0;
  if (![unmodified getCString:withoutOption
                    maxLength:capacity
                     encoding:NSUTF8StringEncoding])
    withoutOption[0] = 0;
  return event.keyCode;
}
int ngterm_key(unsigned *mods, unsigned *plain, unsigned *command, char *text,
               char *withoutOption, unsigned capacity) {
  return ngterm_key_event((__bridge void *)NSApp.currentEvent, mods, plain,
                          command, text, withoutOption, capacity);
}

@interface NGNotifications : NSObject <UNUserNotificationCenterDelegate>
@property(nonatomic) void (*clicked)(const char *);
@end
@implementation NGNotifications
- (void)userNotificationCenter:(UNUserNotificationCenter *)center
    didReceiveNotificationResponse:(UNNotificationResponse *)response
             withCompletionHandler:(void (^)(void))completion {
  if (self.clicked)
    self.clicked(response.notification.request.identifier.UTF8String);
  completion();
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center
       willPresentNotification:(UNNotification *)note
         withCompletionHandler:
             (void (^)(UNNotificationPresentationOptions))completion {
  completion(UNNotificationPresentationOptionBanner |
             UNNotificationPresentationOptionList);
}
@end
static NGNotifications *notifications;
bool ngterm_notifications_start(void (*clicked)(const char *)) {
  // UNUserNotificationCenter requires a bundle identity; a bare dev binary must
  // not call it.
  if (!NSBundle.mainBundle.bundleIdentifier)
    return false;
  notifications = [NGNotifications new];
  notifications.clicked = clicked;
  UNUserNotificationCenter *center =
      UNUserNotificationCenter.currentNotificationCenter;
  center.delegate = notifications;
  [center requestAuthorizationWithOptions:UNAuthorizationOptionAlert |
                                          UNAuthorizationOptionBadge
                        completionHandler:^(BOOL granted, NSError *error){
                        }];
  return true;
}
void ngterm_notify(const char *identity, const char *title, const char *body) {
  UNMutableNotificationContent *content = [UNMutableNotificationContent new];
  content.title = @(title);
  content.body = @(body);
  [UNUserNotificationCenter.currentNotificationCenter
      addNotificationRequest:[UNNotificationRequest
                                 requestWithIdentifier:@(identity)
                                               content:content
                                               trigger:nil]
       withCompletionHandler:nil];
}
void ngterm_withdraw(const char *identity) {
  NSArray *ids = @[ @(identity) ];
  [UNUserNotificationCenter.currentNotificationCenter
      removeDeliveredNotificationsWithIdentifiers:ids];
  [UNUserNotificationCenter.currentNotificationCenter
      removePendingNotificationRequestsWithIdentifiers:ids];
}

#import <WebKit/WebKit.h>
#import <objc/message.h>
#import <objc/runtime.h>
@interface NGWebDelegate : NSObject <WKNavigationDelegate, WKUIDelegate,
                                     WKScriptMessageHandlerWithReply>
@property(nonatomic, weak) WKWebView *view;
@property(nonatomic) void *context;
@property(nonatomic) bool (*decide)
    (void *, const char *, bool, bool, bool, uint64_t);
@property(nonatomic) void (*report)(void *, const char *);
@property(nonatomic) bool (*key)(void *, unsigned);
@property(nonatomic) uint64_t navigation;
@property(nonatomic) NSInteger httpStatus;
@property(nonatomic, strong) id<WKNavigationDelegate>
    previousNavigationDelegate;
@property(nonatomic, strong)
    NSMapTable<WKNavigation *, NSNumber *> *navigationIDs;
@property(nonatomic, strong) NSMutableSet<NSNumber *> *interrupted;
@property(nonatomic, strong) NSMutableDictionary<NSNumber *, id> *replies;
@property(nonatomic, strong) NSTimer *focusTimer;
@property(nonatomic) BOOL focused;
@end
@implementation NGWebDelegate
- (void)emit:(NSDictionary *)value {
  NSData *json = [NSJSONSerialization dataWithJSONObject:value
                                                 options:0
                                                   error:nil];
  if (self.context && json)
    self.report(self.context,
                [[NSString alloc] initWithData:json
                                      encoding:NSUTF8StringEncoding]
                    .UTF8String);
}
- (void)observeValueForKeyPath:(NSString *)key
                      ofObject:(id)object
                        change:(NSDictionary *)change
                       context:(void *)context {
  WKWebView *view = self.view;
  if ([key isEqualToString:@"title"])
    [self emit:@{@"kind" : @"title", @"title" : view.title ?: @""}];
  else if ([key isEqualToString:@"URL"])
    [self emit:@{
      @"kind" : @"source",
      @"url" : view.URL.absoluteString ?: @"about:blank"
    }];
  else
    [self emit:@{
      @"kind" : @"history",
      @"back" : @(view.canGoBack),
      @"forward" : @(view.canGoForward)
    }];
}
- (void)webView:(WKWebView *)view
    decidePolicyForNavigationAction:(WKNavigationAction *)action
                    decisionHandler:(void (^)(WKNavigationActionPolicy))reply {
  BOOL main = action.targetFrame.isMainFrame;
  if (!action.targetFrame) {
    [self emit:@{
      @"kind" : @"new",
      @"url" : action.request.URL.absoluteString ?: @"",
      @"user" : @(action.navigationType == WKNavigationTypeLinkActivated)
    }];
    reply(WKNavigationActionPolicyCancel);
    return;
  }
  if (main) {
    self.navigation++;
    self.httpStatus = 0;
  }
  BOOL allow = self.decide(
      self.context, action.request.URL.absoluteString.UTF8String ?: "", main,
      action.navigationType == WKNavigationTypeLinkActivated, false,
      self.navigation);
  reply(allow ? WKNavigationActionPolicyAllow : WKNavigationActionPolicyCancel);
}
- (void)webView:(WKWebView *)view
    decidePolicyForNavigationResponse:(WKNavigationResponse *)response
                      decisionHandler:
                          (void (^)(WKNavigationResponsePolicy))reply {
  if (response.forMainFrame)
    self.httpStatus = [response.response isKindOfClass:NSHTTPURLResponse.class]
                          ? ((NSHTTPURLResponse *)response.response).statusCode
                          : 0;
  if (!response.canShowMIMEType) {
    if (response.forMainFrame) {
      [self.interrupted addObject:@(self.navigation)];
      [self emit:@{
        @"kind" : @"download",
        @"url" : response.response.URL.absoluteString ?: @""
      }];
      [self emit:@{
        @"kind" : @"interrupted",
        @"id" : @(self.navigation),
        @"reason" : @"download refused"
      }];
    }
    reply(WKNavigationResponsePolicyCancel);
    return;
  }
  BOOL allow = self.decide(
      self.context, response.response.URL.absoluteString.UTF8String ?: "",
      response.forMainFrame, false, true, self.navigation);
  reply(allow ? WKNavigationResponsePolicyAllow
              : WKNavigationResponsePolicyCancel);
}
- (void)webView:(WKWebView *)view
    didStartProvisionalNavigation:(WKNavigation *)navigation {
  [self.navigationIDs setObject:@(self.navigation) forKey:navigation];
  [self emit:@{@"kind" : @"started"}];
}
- (void)webView:(WKWebView *)view
    didCommitNavigation:(WKNavigation *)navigation {
  // Wry's delegate owns its pending-script queue. Preserve that bookkeeping.
  if ([self.previousNavigationDelegate respondsToSelector:_cmd])
    [self.previousNavigationDelegate webView:view
                         didCommitNavigation:navigation];
  [self emit:@{@"kind" : @"commit"}];
}
- (uint64_t)identifier:(WKNavigation *)navigation {
  return [self.navigationIDs objectForKey:navigation].unsignedLongLongValue
             ?: self.navigation;
}
- (void)webView:(WKWebView *)view
    didFinishNavigation:(WKNavigation *)navigation {
  [self emit:@{
    @"kind" : @"complete",
    @"id" : @([self identifier:navigation]),
    @"success" : @YES,
    @"status" : @0,
    @"http" : @(self.httpStatus)
  }];
}
- (void)webView:(WKWebView *)view
    didFailNavigation:(WKNavigation *)navigation
            withError:(NSError *)error {
  NSNumber *identifier = @([self identifier:navigation]);
  if ([self.interrupted containsObject:identifier]) {
    [self.interrupted removeObject:identifier];
    return;
  }
  if ([error.domain isEqual:NSURLErrorDomain] &&
      error.code == NSURLErrorCancelled) {
    [self emit:@{
      @"kind" : @"complete",
      @"id" : identifier,
      @"success" : @NO,
      @"status" : @(-999),
      @"http" : @0
    }];
  } else {
    [self emit:@{
      @"kind" : @"failure",
      @"id" : identifier,
      @"reason" : error.localizedDescription ?: @"the page could not be loaded"
    }];
  }
}
- (void)webView:(WKWebView *)view
    didFailProvisionalNavigation:(WKNavigation *)navigation
                       withError:(NSError *)error {
  [self webView:view didFailNavigation:navigation withError:error];
}
- (void)webViewWebContentProcessDidTerminate:(WKWebView *)view {
  [self emit:@{@"kind" : @"process"}];
}
- (void)webViewDidClose:(WKWebView *)view {
  [self emit:@{@"kind" : @"close"}];
}
- (WKWebView *)webView:(WKWebView *)view
    createWebViewWithConfiguration:(WKWebViewConfiguration *)config
               forNavigationAction:(WKNavigationAction *)action
                    windowFeatures:(WKWindowFeatures *)features {
  return nil;
}
- (void)userContentController:(WKUserContentController *)controller
      didReceiveScriptMessage:(WKScriptMessage *)message
                 replyHandler:(void (^)(id, NSString *))reply {
  if (!message.frameInfo.isMainFrame ||
      ![message.body isKindOfClass:NSString.class]) {
    reply(nil, @"only the main frame may call the bridge");
    return;
  }
  NSDictionary *request = [NSJSONSerialization
      JSONObjectWithData:[message.body dataUsingEncoding:NSUTF8StringEncoding]
                 options:0
                   error:nil];
  NSNumber *identifier = request[@"id"];
  if (![identifier isKindOfClass:NSNumber.class] || self.replies[identifier] ||
      self.replies.count >= 1024) {
    reply(nil, @"invalid bridge request");
    return;
  }
  self.replies[identifier] = [reply copy];
  [self emit:@{@"kind" : @"message", @"body" : message.body}];
}
- (void)webView:(WKWebView *)view
    runJavaScriptAlertPanelWithMessage:(NSString *)message
                      initiatedByFrame:(WKFrameInfo *)frame
                     completionHandler:(void (^)(void))reply {
  reply();
}
- (void)webView:(WKWebView *)view
    runJavaScriptConfirmPanelWithMessage:(NSString *)message
                        initiatedByFrame:(WKFrameInfo *)frame
                       completionHandler:(void (^)(BOOL))reply {
  reply(NO);
}
- (void)webView:(WKWebView *)view
    runJavaScriptTextInputPanelWithPrompt:(NSString *)prompt
                              defaultText:(NSString *)text
                         initiatedByFrame:(WKFrameInfo *)frame
                        completionHandler:(void (^)(NSString *))reply {
  reply(nil);
}
- (void)webView:(WKWebView *)view
    runOpenPanelWithParameters:(WKOpenPanelParameters *)parameters
              initiatedByFrame:(WKFrameInfo *)frame
             completionHandler:(void (^)(NSArray<NSURL *> *))reply {
  reply(nil);
}
- (void)webView:(WKWebView *)view
    requestMediaCapturePermissionForOrigin:(WKSecurityOrigin *)origin
                          initiatedByFrame:(WKFrameInfo *)frame
                                      type:(WKMediaCaptureType)type
                           decisionHandler:
                               (void (^)(WKPermissionDecision))reply {
  reply(WKPermissionDecisionDeny);
}
@end
static char delegateKey;
static NSMutableDictionary<NSValue *, NSValue *> *pageKeyImplementations;
static void cancelOperation(id self, SEL selector, id sender) {
} // Esc belongs to the page, including in fullscreen.
static void ngtermPageKeyDown(id self, SEL selector, NSEvent *event) {
  NGWebDelegate *delegate = objc_getAssociatedObject(self, &delegateKey);
  if (delegate && delegate.context &&
      delegate.key(delegate.context, event.keyCode))
    return;
  // Call the saved implementation directly. KVO can change the live isa at any
  // time.
  for (Class cls = object_getClass(self); cls; cls = class_getSuperclass(cls)) {
    NSValue *saved = pageKeyImplementations[
        [NSValue valueWithPointer:(__bridge const void *)cls]];
    if (saved) {
      ((void (*)(id, SEL, NSEvent *))saved.pointerValue)(self, selector, event);
      return;
    }
  }
}
void *ngterm_web_attach(void *pointer, void *context,
                        bool (*decide)(void *, const char *, bool, bool, bool,
                                       uint64_t),
                        void (*report)(void *, const char *),
                        bool (*key)(void *, unsigned)) {
  WKWebView *view = (__bridge WKWebView *)pointer;
  NGWebDelegate *delegate = [NGWebDelegate new];
  // Keep AppKit's KVO subclass intact: replacing isa invalidates existing
  // observations in Wry. Install once on the declared class, saving its
  // original method rather than computing super from isa.
  Class pageClass = [view class];
  if (!pageKeyImplementations)
    pageKeyImplementations = [NSMutableDictionary new];
  NSValue *classKey =
      [NSValue valueWithPointer:(__bridge const void *)pageClass];
  if (!pageKeyImplementations[classKey]) {
    IMP original =
        class_getMethodImplementation(pageClass, @selector(keyDown:));
    pageKeyImplementations[classKey] = [NSValue valueWithPointer:original];
    class_replaceMethod(pageClass, @selector(keyDown:), (IMP)ngtermPageKeyDown,
                        "v@:@");
    class_replaceMethod(pageClass, @selector(cancelOperation:),
                        (IMP)cancelOperation, "v@:@");
  }
  objc_setAssociatedObject(view, &delegateKey, delegate,
                           OBJC_ASSOCIATION_ASSIGN);
  delegate.navigationIDs = [NSMapTable weakToStrongObjectsMapTable];
  delegate.interrupted = [NSMutableSet new];
  delegate.replies = [NSMutableDictionary new];
  delegate.previousNavigationDelegate = view.navigationDelegate;
  view.configuration.preferences.elementFullscreenEnabled = NO;
  [view.configuration.userContentController
      addScriptMessageHandlerWithReply:delegate
                          contentWorld:WKContentWorld.pageWorld
                                  name:@"ngtermBridge"];
  delegate.key = key;
  delegate.view = view;
  delegate.context = context;
  delegate.decide = decide;
  delegate.report = report;
  view.navigationDelegate = delegate;
  view.UIDelegate = delegate;
  __weak NGWebDelegate *weakDelegate = delegate;
  delegate.focusTimer = [NSTimer
      scheduledTimerWithTimeInterval:0.1
                             repeats:YES
                               block:^(NSTimer *timer) {
                                 NGWebDelegate *owner = weakDelegate;
                                 WKWebView *page = owner.view;
                                 NSResponder *responder =
                                     page.window.firstResponder;
                                 BOOL focused =
                                     [responder isKindOfClass:NSView.class] &&
                                     [(NSView *)responder isDescendantOf:page];
                                 if (owner && owner.focused != focused) {
                                   owner.focused = focused;
                                   [owner emit:@{
                                     @"kind" : @"focus",
                                     @"focused" : @(focused)
                                   }];
                                 }
                               }];
  for (NSString *key in @[ @"URL", @"title", @"canGoBack", @"canGoForward" ])
    [view addObserver:delegate forKeyPath:key options:0 context:nil];
  return (__bridge_retained void *)delegate;
}
void ngterm_web_detach(void *pointer) {
  NGWebDelegate *delegate = (__bridge_transfer NGWebDelegate *)pointer;
  WKWebView *view = delegate.view;
  [delegate.focusTimer invalidate];
  for (NSString *key in @[ @"URL", @"title", @"canGoBack", @"canGoForward" ])
    [view removeObserver:delegate forKeyPath:key];
  [view.configuration.userContentController
      removeScriptMessageHandlerForName:@"ngtermBridge"
                           contentWorld:WKContentWorld.pageWorld];
  for (void (^reply)(id, NSString *) in delegate.replies.allValues)
    reply(nil, @"page closed");
  [delegate.replies removeAllObjects];
  view.navigationDelegate = delegate.previousNavigationDelegate;
  view.UIDelegate = nil;
  delegate.context = NULL;
  objc_setAssociatedObject(view, &delegateKey, nil, OBJC_ASSOCIATION_ASSIGN);
}
void ngterm_web_reparent(void *pointer, void *parent) {
  WKWebView *view = (__bridge WKWebView *)pointer;
  [view removeFromSuperview];
  [(__bridge NSView *)parent addSubview:view];
}
void ngterm_web_focus_parent(void *pointer) {
  NSView *view = (__bridge NSView *)pointer;
  [view.window makeFirstResponder:view.superview];
}
void ngterm_web_eval(void *pointer, const char *script) {
  [(__bridge WKWebView *)pointer evaluateJavaScript:@(script)
                                  completionHandler:nil];
}
void ngterm_web_reply(void *pointer, const char *json) {
  NGWebDelegate *delegate =
      objc_getAssociatedObject((__bridge id)pointer, &delegateKey);
  NSDictionary *message = [NSJSONSerialization
      JSONObjectWithData:[@(json) dataUsingEncoding:NSUTF8StringEncoding]
                 options:0
                   error:nil];
  NSNumber *identifier = message[@"id"];
  void (^reply)(id, NSString *) = delegate.replies[identifier];
  if (reply) {
    [delegate.replies removeObjectForKey:identifier];
    reply(message, nil);
  }
}
void ngterm_web_theme(void *pointer, const char *script) {
  WKUserContentController *controller =
      [(__bridge WKWebView *)pointer configuration].userContentController;
  NSArray *scripts = [controller.userScripts mutableCopy];
  [controller removeAllUserScripts];
  WKUserScript *theme = [[WKUserScript alloc]
        initWithSource:[@"/* ngterm-theme */\n"
                           stringByAppendingString:@(script)]
         injectionTime:WKUserScriptInjectionTimeAtDocumentStart
      forMainFrameOnly:YES];
  BOOL replaced = NO;
  for (WKUserScript *old in scripts) {
    if ([old.source hasPrefix:@"/* ngterm-theme */"]) {
      [controller addUserScript:theme];
      replaced = YES;
    } else
      [controller addUserScript:old];
  }
  if (!replaced)
    [controller addUserScript:theme];
}

void ngterm_clipboard_html(const char *text, const char *html) {
  NSPasteboard *board = NSPasteboard.generalPasteboard;
  [board clearContents];
  [board setString:@(html) forType:NSPasteboardTypeHTML];
  if (text)
    [board setString:@(text) forType:NSPasteboardTypeString];
}
void ngterm_web_clear(const unsigned char *uuid, void *context,
                      void (*done)(void *)) {
  if (@available(macOS 14.0, *)) {
    NSUUID *identifier = [[NSUUID alloc] initWithUUIDBytes:uuid];
    WKWebsiteDataStore *store =
        [WKWebsiteDataStore dataStoreForIdentifier:identifier];
    [store removeDataOfTypes:WKWebsiteDataStore.allWebsiteDataTypes
               modifiedSince:NSDate.distantPast
           completionHandler:^{
             done(context);
           }];
  }
}
bool ngterm_web_supported(void) {
  if (@available(macOS 14.0, *))
    return true;
  return false;
}
#import <Metal/Metal.h>
bool ngterm_metal_available(void) {
  return MTLCreateSystemDefaultDevice() != nil;
}
void ngterm_choose_folder(const char *title, const char *accept,
                          const char *directory, void *context,
                          void (*reply)(void *, const char *)) {
  NSString *message = @(title), *prompt = @(accept),
           *folder = directory ? @(directory) : nil;
  // Opening a sheet may re-enter GPUI; defer until the current callback has
  // released its borrow.
  dispatch_async(dispatch_get_main_queue(), ^{
    NSOpenPanel *panel = NSOpenPanel.openPanel;
    panel.canChooseFiles = NO;
    panel.canChooseDirectories = YES;
    panel.allowsMultipleSelection = NO;
    panel.message = message;
    panel.prompt = prompt;
    if (folder)
      panel.directoryURL = [NSURL fileURLWithPath:folder isDirectory:YES];
    void (^complete)(NSModalResponse) = ^(NSModalResponse response) {
      reply(context,
            response == NSModalResponseOK ? panel.URL.path.UTF8String : NULL);
    };
    if (NSApp.keyWindow)
      [panel beginSheetModalForWindow:NSApp.keyWindow
                    completionHandler:complete];
    else
      [panel beginWithCompletionHandler:complete];
  });
}

// AppKit routes Edit through the current first responder, including a focused
// WKWebView.
@interface NGMenuTarget : NSObject
@property void (*clicked)(NSUInteger);
- (void)activate:(NSMenuItem *)item;
@end
@implementation NGMenuTarget
- (void)activate:(NSMenuItem *)item {
  self.clicked(item.tag);
}
@end
static NGMenuTarget *menuTarget;
static NSMenu *ngtermMenu(NSArray *entries) {
  NSMenu *menu = [NSMenu new];
  menu.autoenablesItems = NO;
  for (NSDictionary *entry in entries) {
    if ([entry[@"separator"] boolValue]) {
      [menu addItem:NSMenuItem.separatorItem];
      continue;
    }
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:entry[@"title"]
                                                  action:NULL
                                           keyEquivalent:@""];
    item.enabled = entry[@"enabled"] ? [entry[@"enabled"] boolValue] : YES;
    if (entry[@"key"]) {
      NSString *key = entry[@"key"];
      NSDictionary *named = @{
        @"enter" : @"\r",
        @"tab" : @"\t",
        @"space" : @" ",
        @"escape" : @"\x1b",
        @"backspace" : @"\x08",
        @"delete" : @"\x7f",
        @"left" : @"\uF702",
        @"right" : @"\uF703",
        @"up" : @"\uF700",
        @"down" : @"\uF701"
      };
      NSString *equivalent = named[key] ?: (key.length == 1 ? key : @"");
      unsigned mods = [entry[@"mods"] unsignedIntValue];
      item.keyEquivalent = (mods & 8) ? equivalent.uppercaseString : equivalent;
      item.keyEquivalentModifierMask =
          ((mods & 1) ? NSEventModifierFlagControl : 0) |
          ((mods & 2) ? NSEventModifierFlagCommand : 0) |
          ((mods & 4) ? NSEventModifierFlagOption : 0) |
          ((mods & 8) ? NSEventModifierFlagShift : 0);
    }
    item.state = [entry[@"checked"] boolValue] ? NSControlStateValueOn
                                               : NSControlStateValueOff;
    if (entry[@"children"]) {
      item.submenu = ngtermMenu(entry[@"children"]);
    } else if (entry[@"tag"]) {
      item.tag = [entry[@"tag"] integerValue];
      item.target = menuTarget;
      item.action = @selector(activate:);
    }
    [menu addItem:item];
  }
  return menu;
}
static void ngtermResponderItem(NSMenu *menu, NSString *title, SEL action,
                                NSString *key, NSEventModifierFlags flags) {
  NSMenuItem *item = [[NSMenuItem alloc]
      initWithTitle:title
             action:action
      keyEquivalent:(flags & NSEventModifierFlagShift) ? key.uppercaseString
                                                       : key];
  item.keyEquivalentModifierMask = flags;
  item.target = nil;
  [menu addItem:item];
}
void ngterm_menus_set(const char *json, void (*clicked)(NSUInteger)) {
  NSDictionary *model = [NSJSONSerialization
      JSONObjectWithData:[@(json) dataUsingEncoding:NSUTF8StringEncoding]
                 options:0
                   error:nil];
  if (!menuTarget)
    menuTarget = [NGMenuTarget new];
  menuTarget.clicked = clicked;
  NSMenu *bar = [NSMenu new];
  NSMenuItem *app = [[NSMenuItem alloc] initWithTitle:@"ngterm"
                                               action:NULL
                                        keyEquivalent:@""];
  app.submenu = ngtermMenu(model[@"app"]);
  // Quit and Settings retain their normal AppKit key equivalents.
  for (NSMenuItem *item in app.submenu.itemArray) {
    if ([item.title hasPrefix:@"Quit"])
      item.keyEquivalent = @"q";
    if ([item.title hasPrefix:@"Settings"])
      item.keyEquivalent = @",";
  }
  [app.submenu addItem:NSMenuItem.separatorItem];
  NSMenuItem *services = [[NSMenuItem alloc] initWithTitle:@"Services"
                                                    action:NULL
                                             keyEquivalent:@""];
  services.submenu = [NSMenu new];
  NSApp.servicesMenu = services.submenu;
  [app.submenu addItem:services];
  ngtermResponderItem(app.submenu, @"Hide ngterm", @selector(hide:), @"h",
                      NSEventModifierFlagCommand);
  ngtermResponderItem(app.submenu, @"Hide Others",
                      @selector(hideOtherApplications:), @"h",
                      NSEventModifierFlagCommand | NSEventModifierFlagOption);
  ngtermResponderItem(app.submenu, @"Show All",
                      @selector(unhideAllApplications:), @"", 0);
  [bar addItem:app];
  for (NSDictionary *entry in model[@"menus"]) {
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:entry[@"title"]
                                                  action:NULL
                                           keyEquivalent:@""];
    item.submenu = ngtermMenu(entry[@"children"]);
    [bar addItem:item];
  }
  NSMenuItem *edit = [[NSMenuItem alloc] initWithTitle:@"Edit"
                                                action:NULL
                                         keyEquivalent:@""];
  edit.submenu = [NSMenu new];
  ngtermResponderItem(edit.submenu, @"Undo", @selector(undo:), @"z",
                      NSEventModifierFlagCommand);
  ngtermResponderItem(edit.submenu, @"Redo", @selector(redo:), @"z",
                      NSEventModifierFlagCommand | NSEventModifierFlagShift);
  [edit.submenu addItem:NSMenuItem.separatorItem];
  ngtermResponderItem(edit.submenu, @"Cut", @selector(cut:), @"x",
                      NSEventModifierFlagCommand);
  ngtermResponderItem(edit.submenu, @"Copy", @selector(copy:), @"c",
                      NSEventModifierFlagCommand);
  ngtermResponderItem(edit.submenu, @"Paste", @selector(paste:), @"v",
                      NSEventModifierFlagCommand);
  ngtermResponderItem(edit.submenu, @"Select All", @selector(selectAll:), @"a",
                      NSEventModifierFlagCommand);
  [bar insertItem:edit atIndex:MIN(2, bar.numberOfItems)];
  NSApp.mainMenu = bar;
}
