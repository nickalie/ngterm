(() => {
  // WPE WebKit's media player reads http, https and blob sources only (WebKitWebSourceGStreamer.cpp): media a file
  // page names beside it, served through the app's schemes, fails as unsupported. Such a failure is caught before the
  // page sees it, the file is read whole through the same scheme, and the element plays it from a blob instead:
  // - a `src`: the blob becomes the element's source, and the play it was asked for (a call, `autoplay`, its controls)
  //   goes on there; a `play()` the failure rejected waits for the rescue instead;
  // - a `<source>`: its own address becomes the blob's, in place, and a selection that ran out of candidates runs
  //   again (WebKit 2.54 resumes none on such a change), in the page's own fallback order; a `play()` that rerun
  //   aborted plays on in it, and a later candidate that loaded meanwhile keeps playing.
  // A read that fails reports the failure the page was spared, once, and a `<source>`'s only once a selection walking
  // the candidates in order would have reached it; a read the page moved on from says nothing. A `pause()` or `load()`
  // aborts the plays asked for before it, as a browser's does, waiting or not. Elements in the document are watched
  // from the window; one a script makes and keeps out of it (`new Audio(url)`) is watched
  // from its making, which only the page's own world sees: the script runs there when the page's scripts run, in the
  // bridge's world when they don't (the parser makes every element then).
  const schemes = ['ngterm-file:', 'ngterm-page:'];
  // the page may replace these later; the rescue keeps the originals
  const read = window.fetch.bind(window);
  const objectUrl = URL.createObjectURL.bind(URL);
  const revoke = URL.revokeObjectURL.bind(URL);
  const nativePlay = HTMLMediaElement.prototype.play;
  const nativePause = HTMLMediaElement.prototype.pause;
  const nativeLoad = HTMLMediaElement.prototype.load;
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

  // the page's latest stop (a `pause()`, a `load()`) of each element still to come: a play asked for before it keeps
  // it, and a stop rejects what it keeps, so a play waiting on a rescue gives up on it
  const stops = new WeakMap();
  const stopOf = (media) => {
    let stop = stops.get(media);
    if (!stop) {
      stop = {};
      stop.aborted = new Promise((_, reject) => {
        stop.abort = reject;
      });
      stop.aborted.catch(() => {});
      stops.set(media, stop);
    }
    return stop;
  };
  const stopped = (media) => {
    stops.get(media)?.abort(failure('AbortError'));
    stops.delete(media);
  };
  // the address each element had when the page last paused it: its `autoplay` stays off until the page loads it
  // again (a `load()`, another address). WebKit's own load turns it back on, a rescue's too, which `settle` undoes.
  const disarmed = new WeakMap();
  const autoplays = (media) => media.autoplay && disarmed.get(media) !== media.src;
  // after a rescue's load of `media`: the play the page wished for before it (`playing`, or an `autoplay` it left on)
  // goes on, and an `autoplay` the page turned off stays off
  const settle = (media, playing, autoplay) => {
    if (playing || autoplay) {
      // a play refused now (no gesture, a format the blob cannot decode) shows on the element's own events
      nativePlay.call(media).catch(() => {});
    } else if (media.autoplay) {
      // paused already: this only turns the load's `autoplay` off again, with no event
      nativePause.call(media);
      disarmed.set(media, media.src);
    }
  };

  // the rescue of each element's `src` under way: its address and the outcome a rejected `play()` waits for
  const rescues = new WeakMap();
  // the address whose read failed for each element, so a `play()` of it only rejects, as a browser's does, until a
  // load fails afresh
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
    unreadable.delete(media);
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
        // the page's wish as it stands now, before the blob's load pauses the element
        const playing = !media.paused;
        const autoplay = autoplays(media);
        playBlob(media, blob);
        rescue.resolve();
        settle(media, playing, autoplay);
      },
      () => {
        if (!settled()) return;
        unreadable.set(media, url);
        rescue.reject(failure('NotSupportedError'));
        media.dispatchEvent(new Event('error'));
      },
    );
  };

  // each `<source>` read under way or failed: its address, its state and whether its failure was told
  const sources = new WeakMap();
  // the blob each `<source>` tried and could not play
  const spent = new WeakMap();
  // whether a `<source>` keeps a selection from reaching the candidates after it: its read is under way, or it plays
  // (or is yet to try) a rescued blob
  const holds = (child) => {
    if (!(child instanceof HTMLSourceElement)) return false;
    const entry = sources.get(child);
    if (entry?.state === 'reading' && entry.url === child.src) return true;
    return played.get(child) === child.src && spent.get(child) !== child.src;
  };
  // whether a selection walking the `<source>`s in order reaches `source`
  const reached = (source, media) => {
    for (const child of media.children) {
      if (child === source) return true;
      if (holds(child)) return false;
    }
    return false;
  };
  // tells the failed reads a selection would have reached by now, in order, each once
  const tell = (media) => {
    for (const child of [...media.children]) {
      if (holds(child)) return;
      const entry = child instanceof HTMLSourceElement ? sources.get(child) : undefined;
      if (entry?.state === 'failed' && !entry.told && entry.url === child.src) {
        entry.told = true;
        child.dispatchEvent(new Event('error'));
      }
    }
  };
  // the `<source>`s a rescue pointed at a blob that no selection has tried since
  const untried = new WeakSet();
  // the elements whose selection a rescue runs again, until it settles: a `play()` it aborts plays on, and the
  // failure of a source the page was already told about is not told again
  const reloading = new WeakSet();
  const rerun = (media) => {
    const playing = !media.paused;
    const autoplay = autoplays(media);
    reloading.add(media);
    const settled = () => reloading.delete(media);
    media.addEventListener('loadedmetadata', settled, { once: true });
    media.addEventListener('error', settled, { once: true });
    nativeLoad.call(media);
    settle(media, playing, autoplay);
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
      if (known.state === 'reading') {
        known.heldIn.add(media);
        return true;
      }
      // a failure held back until a selection reaches it is told by the first one that does
      if (!known.told) {
        if (!reached(source, media)) return true;
        known.told = true;
        return false;
      }
      // a failed read was told once; again only when the page ran the selection itself
      return reloading.has(media);
    }
    // `heldIn`: the elements whose selection reached it while its read was under way, and held its failure back
    const entry = { url, state: 'reading', told: false, heldIn: new Set([media]) };
    sources.set(source, entry);
    // the elements a settled read speaks for: the one it failed in, which a `<source>` the page moved or removed
    // holds back nothing in any more, and the one it is in now when that one's selection reached it meanwhile (one
    // whose selection has yet to reach it tells the failure itself, when it does)
    const settledIn = () => {
      const now = source.parentElement;
      return now instanceof HTMLMediaElement && now !== media && entry.heldIn.has(now) ? [media, now] : [media];
    };
    readWhole(url).then(
      (blob) => {
        if (sources.get(source) !== entry) return;
        sources.delete(source);
        const elements = settledIn();
        // unless the page gave the `<source>` another address meanwhile
        if (source.src === url) {
          // in place, so the page's own order, a later change of its address and its removal all hold
          playBlob(source, blob);
          untried.add(source);
          if (source.parentElement instanceof HTMLMediaElement) selectionEnded(source.parentElement);
        }
        for (const element of elements) tell(element);
      },
      () => {
        if (sources.get(source) !== entry) return;
        entry.state = 'failed';
        // told once no earlier candidate holds the selection back from it, by this walk or a later one
        for (const element of settledIn()) tell(element);
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
      // a rescued blob that cannot play holds the selection no longer
      if (played.get(target) === target.src) spent.set(target, target.src);
      // the last candidate's failure ends the selection, whose state settles once this event is through
      setTimeout(() => selectionEnded(media), 0);
      if (fromApp(target.src) && rescueSource(target, media)) event.stopImmediatePropagation();
    }
  };
  addEventListener('error', failed, true);

  // a `src`'s failure rejects a `play()` waiting on the element, and one called while the read is under way, before
  // the blob is in place: such a call waits for the rescue and plays there (WebKit rejects the promise before the
  // failure's `error` event runs, a microtask against a task, so the rescue may start here). A `play()` a rescue's
  // rerun of the selection aborted plays on in the next one, however many reruns follow. A `play()` of an element
  // whose read already failed, told to the page, only rejects; one after a fresh load gets a fresh rescue. A stop of
  // the page's after the call aborts it, waiting or not.
  const play = function play() {
    const stop = stopOf(this);
    const told =
      this.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED && unreadable.get(this) === this.currentSrc;
    return nativePlay.call(this).catch((error) => {
      const current = stops.get(this) === stop;
      if (error?.name === 'NotSupportedError' && !told) {
        const url = this.currentSrc;
        if (this.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED && fromApp(url)) rescueSrc(this, url);
        const rescue = rescues.get(this);
        if (rescue && !current) throw failure('AbortError');
        if (rescue) {
          return Promise.race([rescue.outcome, stop.aborted]).then(() => {
            // a stop in the same checkpoint as the rescue's outcome (a page's observer of the blob's address) may
            // lose the race to it
            if (stops.get(this) !== stop) throw failure('AbortError');
            return play.call(this);
          });
        }
      }
      if (error?.name === 'AbortError' && reloading.has(this) && current) return play.call(this);
      throw error;
    });
  };
  HTMLMediaElement.prototype.play = play;
  HTMLMediaElement.prototype.pause = function pause() {
    stopped(this);
    disarmed.set(this, this.src);
    return nativePause.call(this);
  };
  HTMLMediaElement.prototype.load = function load() {
    stopped(this);
    disarmed.delete(this);
    return nativeLoad.call(this);
  };

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
