(() => {
  // WPE WebKit's media player reads http, https and blob sources only (WebKitWebSourceGStreamer.cpp): media a file
  // page names beside it, served through the app's schemes, fails as unsupported. Such a failure is caught before the
  // page sees it, the file is read whole through the same scheme, and the element plays it from a blob instead:
  // - a `src`: the blob becomes the element's source, and the play it was asked for (a call, `autoplay`, its controls)
  //   goes on there; a `play()` the failure rejected waits for the rescue instead;
  // - a `<source>`: its own address becomes the blob's, in place, and a selection that ran out of candidates runs
  //   again (WebKit 2.54 resumes none on such a change), in the page's own fallback order; a `play()` that rerun
  //   aborted plays on in it, and a later candidate that loaded meanwhile keeps playing.
  // A read that fails reports the failure the page was spared, once; a read the page moved on from says nothing. Elements in
  // the document are watched from the window; one a script makes and keeps out of it (`new Audio(url)`) is watched
  // from its making, which only the page's own world sees: the script runs there when the page's scripts run, in the
  // bridge's world when they don't (the parser makes every element then).
  const schemes = ['ngterm-file:', 'ngterm-page:'];
  // the page may replace these later; the rescue keeps the originals
  const read = window.fetch.bind(window);
  const objectUrl = URL.createObjectURL.bind(URL);
  const revoke = URL.revokeObjectURL.bind(URL);
  const nativePlay = HTMLMediaElement.prototype.play;
  const create = Document.prototype.createElement;
  const createNS = Document.prototype.createElementNS;
  const fromApp = (url) => {
    try {
      return schemes.includes(new URL(url).protocol);
    } catch {
      return false;
    }
  };
  const readWhole = (url) =>
    read(url).then((response) => {
      if (!response.ok) throw new Error(response.statusText);
      return response.blob();
    });
  // a blob's address lives as long as the element that plays it
  const released = new FinalizationRegistry(revoke);
  const blobFor = (owner, blob) => {
    const address = objectUrl(blob);
    released.register(owner, address);
    return address;
  };
  const failure = (name) => new DOMException('the media could not be read', name);

  // the rescue of each element's `src` under way: its address and the outcome a rejected `play()` waits for
  const rescues = new WeakMap();
  // the address whose read failed for each element, so a `play()` of it only rejects, as a browser's does
  const unreadable = new WeakMap();
  // the blob each element (a media element's `src`, a `<source>`) plays, let go when a later rescue replaces it
  const played = new WeakMap();
  const playBlob = (element, blob) => {
    const address = blobFor(element, blob);
    const previous = played.get(element);
    if (previous) revoke(previous);
    played.set(element, address);
    element.src = address;
  };
  const rescueSrc = (media, url) => {
    const current = rescues.get(media);
    // the same address failing again while its read is under way waits for that read
    if (current?.url === url) return;
    // another address: the page moved on, and the earlier read no longer speaks for the element
    current?.reject(failure('AbortError'));
    const resume = !media.paused || media.autoplay;
    const rescue = { url };
    rescue.outcome = new Promise((resolve, reject) => {
      rescue.resolve = resolve;
      rescue.reject = reject;
    });
    rescue.outcome.catch(() => {});
    rescues.set(media, rescue);
    const settled = () => {
      if (rescues.get(media) !== rescue) return false;
      rescues.delete(media);
      if (media.src === url) return true;
      rescue.reject(failure('AbortError'));
      return false;
    };
    readWhole(url).then(
      (blob) => {
        if (!settled()) return;
        playBlob(media, blob);
        rescue.resolve();
        // a play refused now (no gesture, a format the blob cannot decode) shows on the element's own events
        if (resume) nativePlay.call(media).catch(() => {});
      },
      () => {
        if (!settled()) return;
        unreadable.set(media, url);
        rescue.reject(failure('NotSupportedError'));
        media.dispatchEvent(new Event('error'));
      },
    );
  };

  // each `<source>` read under way or failed: its address and its state
  const sources = new WeakMap();
  // the `<source>`s a rescue pointed at a blob that no selection has tried since
  const untried = new WeakSet();
  // the elements whose selection a rescue runs again, until it settles: a `play()` it aborts plays on, and the
  // failure of a source the page was already told about is not told again
  const reloading = new WeakSet();
  const rerun = (media) => {
    const resume = !media.paused || media.autoplay;
    reloading.add(media);
    const settled = () => reloading.delete(media);
    media.addEventListener('loadedmetadata', settled, { once: true });
    media.addEventListener('error', settled, { once: true });
    media.load();
    if (resume) nativePlay.call(media).catch(() => {});
  };
  // a selection that ran out of candidates (WebKit 2.54 resumes none when a `<source>` changes) runs again while a
  // rescued `<source>` waits untried; one that ends with none waiting has settled
  const selectionEnded = (media) => {
    if (media.hasAttribute('src') || media.networkState !== HTMLMediaElement.NETWORK_NO_SOURCE) return;
    const waiting = [...media.children].filter((child) => untried.has(child));
    if (waiting.length === 0) {
      reloading.delete(media);
      return;
    }
    for (const source of waiting) untried.delete(source);
    rerun(media);
  };
  // whether the failure of `source` is stopped: false lets a failure the page should see through
  const rescueSource = (source, media) => {
    const url = source.src;
    const known = sources.get(source);
    if (known?.url === url) {
      if (known.state === 'reading') return true;
      // a failed read was told once; again only when the page ran the selection itself
      return reloading.has(media);
    }
    const entry = { url, state: 'reading' };
    sources.set(source, entry);
    readWhole(url).then(
      (blob) => {
        if (sources.get(source) !== entry) return;
        sources.delete(source);
        // the page gave the `<source>` another address meanwhile
        if (source.src !== url) return;
        // in place, so the page's own order, a later change of its address and its removal all hold
        playBlob(source, blob);
        untried.add(source);
        if (source.parentElement instanceof HTMLMediaElement) selectionEnded(source.parentElement);
      },
      () => {
        if (sources.get(source) !== entry) return;
        entry.state = 'failed';
        if (source.src === url) source.dispatchEvent(new Event('error'));
      },
    );
    return true;
  };

  const failed = (event) => {
    if (!event.isTrusted) return;
    const target = event.target;
    if (target instanceof HTMLMediaElement) {
      if (target.error?.code !== MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED || !fromApp(target.currentSrc)) return;
      event.stopImmediatePropagation();
      rescueSrc(target, target.currentSrc);
    } else if (target instanceof HTMLSourceElement && target.parentElement instanceof HTMLMediaElement) {
      const media = target.parentElement;
      // the last candidate's failure ends the selection, whose state settles once this event is through
      setTimeout(() => selectionEnded(media), 0);
      if (fromApp(target.src) && rescueSource(target, media)) event.stopImmediatePropagation();
    }
  };
  addEventListener('error', failed, true);

  // a `src`'s failure rejects a `play()` waiting on the element, and one called while the read is under way, before
  // the blob is in place: such a call waits for the rescue and plays there (WebKit rejects the promise before the
  // failure's `error` event runs, a microtask against a task, so the rescue may start here). A `play()` a rescue's
  // rerun of the selection aborted plays on in the next one, however many reruns follow.
  const play = function play() {
    return nativePlay.call(this).catch((error) => {
      if (error?.name === 'NotSupportedError') {
        const url = this.currentSrc;
        const readable = fromApp(url) && unreadable.get(this) !== url;
        if (this.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED && readable) rescueSrc(this, url);
        const rescue = rescues.get(this);
        if (rescue) return rescue.outcome.then(() => play.call(this));
      }
      if (error?.name === 'AbortError' && reloading.has(this)) return play.call(this);
      throw error;
    });
  };
  HTMLMediaElement.prototype.play = play;

  // an element made by a script, watched before the script can listen on it or give it a source
  const watch = (element) => {
    if (element instanceof HTMLMediaElement) element.addEventListener('error', failed, true);
    return element;
  };
  window.Audio = new Proxy(window.Audio, {
    construct: (target, args, made) => watch(Reflect.construct(target, args, made)),
  });
  Document.prototype.createElement = function createElement(...args) {
    return watch(create.apply(this, args));
  };
  Document.prototype.createElementNS = function createElementNS(...args) {
    return watch(createNS.apply(this, args));
  };
})();
