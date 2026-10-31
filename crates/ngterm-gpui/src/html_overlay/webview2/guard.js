(() => {
  const call = Function.prototype.call;
  const bound = (fn) => call.bind(fn);
  const inputType = bound(Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'type').get);
  const composedPath = bound(Event.prototype.composedPath);
  const preventDefault = bound(Event.prototype.preventDefault);
  const stopImmediatePropagation = bound(Event.prototype.stopImmediatePropagation);
  const listen = bound(EventTarget.prototype.addEventListener);
  const eventType = bound(Object.getOwnPropertyDescriptor(Event.prototype, 'type').get);
  const define = Object.defineProperty;
  const isFile = (target) => {
    try { return inputType(target) === 'file'; } catch (error) { return false; }
  };
  const guardClick = (event) => {
    const path = composedPath(event);
    for (let index = 0; index < path.length; index += 1) {
      if (isFile(path[index])) { preventDefault(event); return; }
    }
  };
  listen(window, 'click', guardClick, true);
  const refuse = () => new DOMException('file choosers are not available', 'NotAllowedError');
  const method = (prototype, name, guarded) => {
    const original = prototype[name];
    if (!original) return;
    const run = bound(original);
    define(prototype, name, {value: {[name](...rest) { return guarded(this, run, rest); }}[name],
      writable: true, enumerable: true, configurable: true});
  };
  method(HTMLElement.prototype, 'click', (target, run) => { if (!isFile(target)) run(target); });
  method(EventTarget.prototype, 'dispatchEvent', (target, run, rest) => {
    let click = false;
    try { click = eventType(rest[0]) === 'click'; } catch (error) {}
    if (click && isFile(target)) return false;
    return rest.length === 0 ? run(target) : run(target, rest[0]);
  });
  method(Element.prototype, 'attachShadow', (target, run, rest) => {
    const root = rest.length === 0 ? run(target) : run(target, rest[0]);
    listen(root, 'click', guardClick, true);
    return root;
  });
  method(HTMLInputElement.prototype, 'showPicker', (target, run) => {
    if (isFile(target)) throw refuse();
    return run(target);
  });
  for (const name of ['showOpenFilePicker', 'showSaveFilePicker', 'showDirectoryPicker']) {
    try { delete window[name]; } catch (error) {}
    try { define(window, name, {value: undefined, configurable: false}); } catch (error) {}
  }
  const clipboardData = bound(Object.getOwnPropertyDescriptor(ClipboardEvent.prototype, 'clipboardData').get);
  const files = bound(Object.getOwnPropertyDescriptor(DataTransfer.prototype, 'files').get);
  const count = bound(Object.getOwnPropertyDescriptor(FileList.prototype, 'length').get);
  listen(window, 'paste', (event) => {
    let pasted = 0;
    try {
      const data = clipboardData(event);
      pasted = data ? count(files(data)) : 0;
    } catch (error) {}
    if (pasted) {
      preventDefault(event);
      stopImmediatePropagation(event);
    }
  }, true);
})();
