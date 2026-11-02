// Run in a disposable desktop. Exercises the native adapter without a Metal
// GPU.
#import <Carbon/Carbon.h>
#import <Cocoa/Cocoa.h>
#import <IOKit/hidsystem/IOLLEvent.h>
#import <WebKit/WebKit.h>
#include <assert.h>
#import <objc/runtime.h>
extern bool ngterm_reduce_transparency(void), ngterm_reduce_motion(void);
extern int ngterm_hotkey(int, unsigned, void (*)(void));
extern void ngterm_clipboard_html(const char *, const char *);
extern unsigned ngterm_key_flags(unsigned long, bool, bool, bool);
extern int ngterm_key_event(void *, unsigned *, unsigned *, unsigned *, char *,
                            char *, unsigned);
extern void *
ngterm_web_attach(void *, void *,
                  bool (*)(void *, const char *, bool, bool, bool, uint64_t),
                  void (*)(void *, const char *), bool (*)(void *, unsigned));
extern void ngterm_web_detach(void *), ngterm_web_focus_parent(void *);
extern void ngterm_web_theme(void *, const char *),
    ngterm_web_reply(void *, const char *);
extern void ngterm_web_reparent(void *, void *),
    ngterm_menus_set(const char *, void (*)(NSUInteger));
static unsigned started, completed, failed, interrupted, downloads, superKeys,
    commits;
static bool loaded, sawTitle, denied, replied, menuTriggered;
static WKWebView *page;
@interface CountingPage : WKWebView
@end
@implementation CountingPage
- (void)keyDown:(NSEvent *)event {
  superKeys++;
}
@end
// Wry's original delegate needs didCommit to flush its pending evaluations.
@interface PreviousDelegate : NSObject <WKNavigationDelegate>
@end
@implementation PreviousDelegate
- (void)webView:(WKWebView *)view
    didCommitNavigation:(WKNavigation *)navigation {
  commits++;
}
@end
@interface Download : NSObject <WKURLSchemeHandler>
@end
@implementation Download
- (void)webView:(WKWebView *)view startURLSchemeTask:(id<WKURLSchemeTask>)task {
  [task didReceiveResponse:[[NSURLResponse alloc] initWithURL:task.request.URL
                                                     MIMEType:@"application/zip"
                                        expectedContentLength:4
                                             textEncodingName:nil]];
  [task didReceiveData:[@"data" dataUsingEncoding:NSUTF8StringEncoding]];
  [task didFinish];
}
- (void)webView:(WKWebView *)view stopURLSchemeTask:(id<WKURLSchemeTask>)task {
}
@end
static void pressed(void) {}
static bool key(void *context, unsigned code) { return false; }
static void menuClicked(NSUInteger tag) { menuTriggered = tag == 42; }
static bool decide(void *context, const char *url, bool main, bool user,
                   bool redirect, uint64_t id) {
  bool allow =
      strncmp(url, "about:blank", 11) == 0 || strncmp(url, "download:", 9) == 0;
  if (!allow && main)
    denied = true;
  return allow;
}
static void report(void *context, const char *json) {
  NSDictionary *value = [NSJSONSerialization
      JSONObjectWithData:[@(json) dataUsingEncoding:NSUTF8StringEncoding]
                 options:0
                   error:nil];
  NSString *kind = value[@"kind"];
  if ([kind isEqual:@"started"])
    started++;
  if ([kind isEqual:@"complete"]) {
    completed++;
    if ([value[@"success"] boolValue])
      loaded = true;
  }
  if ([kind isEqual:@"failure"])
    failed++;
  if ([kind isEqual:@"interrupted"])
    interrupted++;
  if ([kind isEqual:@"download"])
    downloads++;
  if ([value[@"title"] isEqual:@"ngterm-smoke"])
    sawTitle = true;
  if ([kind isEqual:@"message"]) {
    ngterm_web_reply((__bridge void *)page, "{\"id\":7,\"result\":42}");
    replied = true;
  }
}
static void pump(double seconds) {
  NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:seconds];
  while (deadline.timeIntervalSinceNow > 0) {
    NSEvent *event;
    while ((event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                       untilDate:NSDate.distantPast
                                          inMode:NSDefaultRunLoopMode
                                         dequeue:YES]))
      [NSApp sendEvent:event];
    [NSApp updateWindows];
    [NSRunLoop.currentRunLoop
        runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
  }
}

static void until(bool *flag) {
  NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:20];
  while (!*flag && deadline.timeIntervalSinceNow > 0)
    pump(.02);
  assert(*flag);
}
static id evaluate(NSString *script) {
  __block bool done = false;
  __block id result;
  [page evaluateJavaScript:script
         completionHandler:^(id value, NSError *error) {
           assert(!error);
           result = value;
           done = true;
         }];
  until(&done);
  return result;
}
static NSEvent *event(unsigned code, NSEventModifierFlags flags,
                      NSString *text) {
  return [NSEvent keyEventWithType:NSEventTypeKeyDown
                          location:NSZeroPoint
                     modifierFlags:flags
                         timestamp:0
                      windowNumber:0
                           context:nil
                        characters:text
       charactersIgnoringModifiers:text
                         isARepeat:NO
                           keyCode:code];
}
static void keyboard(void) {
  assert((ngterm_key_flags(NSEventModifierFlagControl | NX_DEVICELCTLKEYMASK,
                           false, false, false) &
          3) == 0);
  assert((ngterm_key_flags(NSEventModifierFlagShift | NX_DEVICELSHIFTKEYMASK,
                           false, false, false) &
          3) == 0);
  assert((ngterm_key_flags(NSEventModifierFlagOption | NX_DEVICERALTKEYMASK,
                           false, true, false) &
          10) == 10);
  unsigned mods, plain, command;
  char text[128], without[128];
  assert(ngterm_key_event((__bridge void *)event(36, 0, @"\r"), &mods, &plain,
                          &command, text, without, 128) == 36);
  assert(!strcmp(text, "\r") && !(mods & 1));
  assert(ngterm_key_event((__bridge void *)event(76, 0, @"\3"), &mods, &plain,
                          &command, text, without, 128) == 76);
  assert(ngterm_key_event(
             (__bridge void *)event(0, NSEventModifierFlagCapsLock, @"A"),
             &mods, &plain, &command, text, without, 128) == 0);
  assert(mods & NSEventModifierFlagCapsLock);
  assert(!strcmp(text, "A"));
  assert(ngterm_key_event(
             (__bridge void *)event(14, NSEventModifierFlagOption, @"´"), &mods,
             &plain, &command, text, without, 128) == 14);
  assert(mods & 4); // US Option+E starts composition; AppKit must receive it.
  assert(!strcmp(without, "e"));
}
static void checkServices(void) {
  @autoreleasepool {
    keyboard();
    assert(ngterm_reduce_transparency() ==
           NSWorkspace.sharedWorkspace
               .accessibilityDisplayShouldReduceTransparency);
    assert(ngterm_reduce_motion() ==
           NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion);
    assert(ngterm_hotkey(40, (1 << 8) | (1 << 12) | (1 << 11), pressed) == 0);
    assert(ngterm_hotkey(-1, 0, pressed) == 0);
    ngterm_clipboard_html("plain", "<b>plain</b>");
    assert([NSPasteboard.generalPasteboard stringForType:NSPasteboardTypeHTML]);
    assert([[NSPasteboard.generalPasteboard
        stringForType:NSPasteboardTypeString] isEqual:@"plain"]);
    ngterm_menus_set(
        "{\"app\":[],\"menus\":[{\"title\":\"File\",\"children\":[{\"title\":"
        "\"Custom\",\"enabled\":true,\"tag\":42,\"key\":\"n\",\"mods\":10}]}]}",
        menuClicked);
    NSMenu *edit = [NSApp.mainMenu itemWithTitle:@"Edit"].submenu;
    assert(edit && [edit itemWithTitle:@"Copy"].action == @selector(copy:));
    assert(![edit itemWithTitle:@"Copy"].target &&
           [[edit itemWithTitle:@"Copy"].keyEquivalent isEqual:@"c"]);
    assert([edit itemWithTitle:@"Redo"].keyEquivalentModifierMask ==
           (NSEventModifierFlagCommand | NSEventModifierFlagShift));
    WKWebViewConfiguration *config = [WKWebViewConfiguration new];
    config.websiteDataStore = WKWebsiteDataStore.nonPersistentDataStore;
    [config setURLSchemeHandler:[Download new] forURLScheme:@"download"];
    [config.userContentController
        addUserScript:
            [[WKUserScript alloc]
                  initWithSource:@"window.keepExistingScripts=99;"
                   injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                forMainFrameOnly:YES]];
    page = [[CountingPage alloc] initWithFrame:NSMakeRect(0, 0, 640, 480)
                                 configuration:config];
    PreviousDelegate *previous = [PreviousDelegate new];
    page.navigationDelegate = previous;
    NSWindow *window =
        [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 640, 480)
                                    styleMask:NSWindowStyleMaskTitled
                                      backing:NSBackingStoreBuffered
                                        defer:NO];
    NSView *first = [[NSView alloc] initWithFrame:page.frame],
           *second = [[NSView alloc] initWithFrame:page.frame];
    [window.contentView addSubview:first];
    [window.contentView addSubview:second];
    ngterm_web_reparent((__bridge void *)page, (__bridge void *)first);
    void *delegate =
        ngterm_web_attach((__bridge void *)page, &loaded, decide, report, key);
    assert(!page.configuration.preferences.elementFullscreenEnabled);
    // KVO already subclasses this view. An ordinary key must call the stored
    // superclass exactly once.
    [page keyDown:event(0, 0, @"a")];
    assert(superKeys == 1);
    ngterm_web_reparent((__bridge void *)page, (__bridge void *)second);
    [window makeFirstResponder:page];
    ngterm_web_focus_parent((__bridge void *)page);
    assert(window.firstResponder == second);
    ngterm_web_theme((__bridge void *)page,
                     "window.ngtermThemeInstalled=true;");
    [page loadHTMLString:
              @"<html><head><title>ngterm-smoke</title></head><body><a "
              @"id='fragment'></a>terminal overlay</body></html>"
                 baseURL:nil];
    until(&loaded);
    until(&sawTitle);
    assert(commits == 1);
    assert([evaluate(
        @"window.ngtermThemeInstalled && window.keepExistingScripts===99")
        boolValue]);
    // Native promises settle even if page code tampers with window event
    // dispatch.
    evaluate(@"window.dispatchEvent=()=>{throw Error('page trap')}; "
             @"window.MessageEvent=()=>{throw Error('page trap')}; "
             @"window.bridgeAnswer=0; "
             @"webkit.messageHandlers.ngtermBridge.postMessage('{\"id\":7}')."
             @"then(r=>window.bridgeAnswer=r.result); true");
    until(&replied);
    pump(.2);
    assert([evaluate(@"window.bridgeAnswer") intValue] == 42);
    // Real Edit key equivalents must reach WebKit through AppKit's responder
    // chain.
    [window makeKeyAndOrderFront:nil];
    [NSApp activate];
    evaluate(@"const editor=document.createElement('div'); editor.id='editor'; "
             @"editor.contentEditable='true'; editor.textContent='page-copy'; "
             @"document.body.append(editor); editor.focus(); "
             @"window.getSelection().selectAllChildren(editor); true");
    [window makeFirstResponder:page];
    pump(.2);
    assert(
        [NSApp.mainMenu performKeyEquivalent:event(45,
                                                   NSEventModifierFlagCommand |
                                                       NSEventModifierFlagShift,
                                                   @"N")]);
    assert(menuTriggered);
    assert([NSApp.mainMenu
        performKeyEquivalent:event(8, NSEventModifierFlagCommand, @"c")]);
    NSDate *copyDeadline = [NSDate dateWithTimeIntervalSinceNow:10];
    while (![[NSPasteboard.generalPasteboard
               stringForType:NSPasteboardTypeString] isEqual:@"page-copy"] &&
           copyDeadline.timeIntervalSinceNow > 0)
      pump(.02);
    assert([[NSPasteboard.generalPasteboard
        stringForType:NSPasteboardTypeString] isEqual:@"page-copy"]);
    assert([NSApp.mainMenu
        performKeyEquivalent:event(7, NSEventModifierFlagCommand, @"x")]);
    pump(.2);
    assert([evaluate(@"document.getElementById('editor').textContent")
        isEqual:@""]);
    [NSPasteboard.generalPasteboard clearContents];
    [NSPasteboard.generalPasteboard setString:@"page-paste"
                                      forType:NSPasteboardTypeString];
    assert([NSApp.mainMenu
        performKeyEquivalent:event(9, NSEventModifierFlagCommand, @"v")]);
    pump(.2);
    assert([evaluate(@"document.getElementById('editor').textContent")
        isEqual:@"page-paste"]);
    assert([NSApp.mainMenu
        performKeyEquivalent:event(0, NSEventModifierFlagCommand, @"a")]);
    pump(.1);
    assert(
        [evaluate(@"window.getSelection().toString()") isEqual:@"page-paste"]);
    assert([NSApp.mainMenu
        performKeyEquivalent:event(6, NSEventModifierFlagCommand, @"z")]);
    pump(.2);
    // WebKit may group cut and paste into one undo step; either result must
    // undo the paste.
    assert(![evaluate(@"document.getElementById('editor').textContent")
        isEqual:@"page-paste"]);
    assert(page.undoManager.canRedo);
    [edit update];
    assert(
        [NSApp.mainMenu performKeyEquivalent:event(6,
                                                   NSEventModifierFlagCommand |
                                                       NSEventModifierFlagShift,
                                                   @"Z")]);
    pump(.2);
    assert([evaluate(@"document.getElementById('editor').textContent")
        isEqual:@"page-paste"]);
    unsigned before = started;
    evaluate(@"location.hash='fragment'; true");
    pump(.3);
    assert(started == before);
    evaluate(
        @"document.body.appendChild(document.createElement('iframe')); true");
    pump(.3);
    assert(started == before);
    // A loadable document would replace the title if refusal were ineffective.
    NSURL *keptURL = page.URL;
    unsigned keptSuccess = completed;
    [page
        loadRequest:[NSURLRequest
                        requestWithURL:
                            [NSURL
                                URLWithString:@"data:text/html,<title>BLOCKED</"
                                              @"title>blocked"]]];
    until(&denied);
    pump(1);
    assert([page.URL isEqual:keptURL] && [page.title isEqual:@"ngterm-smoke"]);
    assert(completed == keptSuccess && started == before && failed == 0);
    [page loadRequest:
              [NSURLRequest
                  requestWithURL:
                      [NSURL URLWithString:@"download://fixture/archive.zip"]]];
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:20];
    while (!downloads && deadline.timeIntervalSinceNow > 0)
      pump(.02);
    pump(.5);
    assert(downloads == 1 && interrupted == 1 && failed == 0);
    assert([page.title isEqual:@"ngterm-smoke"]);
    ngterm_web_detach(delegate);
    assert(page.navigationDelegate == previous);
    puts("PASS: native keys/KVO superclass, settings, hotkey, clipboard, "
         "Edit shortcuts and model menu actions, load/theme/delegate "
         "forwarding, native bridge replies, "
         "reparent focus, same-document/subframe navigation, effective "
         "refusal, download interruption");
  }
}
int main(void) {
  @autoreleasepool {
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    // AppKit activation and nil-target actions require NSApplication's running
    // event loop.
    [NSTimer scheduledTimerWithTimeInterval:1
                                    repeats:NO
                                      block:^(NSTimer *timer) {
                                        checkServices();
                                        exit(0);
                                      }];
    [NSApp run];
  }
  return 0;
}
