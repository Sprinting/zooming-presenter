// Zebra web player: the page side. The wasm module (crates/zebra-web)
// presents the deck and draws each frame; this script feeds it the
// document, fonts and pictures, turns keys, clicks, wheel and touch into
// its commands, copies its frames to the canvas, plays video and audio in
// <video>/<audio> elements laid over the canvas, and runs the control bar.
(function () {
'use strict';
var $ = function (id) { return document.getElementById(id); };
var M = JSON.parse($('zw-manifest').textContent);
var L = M.labels;
var stage = $('zw-stage'), canvas = $('zw-canvas'), mediaLayer = $('zw-media'), blank = $('zw-blank');
var X = null; // wasm exports
var enc = new TextEncoder(), dec = new TextDecoder();
var started = performance.now();
var timings = { start: started };
window.zebraPlayer = { timings: timings };

function fail(msg) {
  var d = document.createElement('div');
  d.id = 'zw-msg';
  d.textContent = msg;
  document.body.appendChild(d);
  window.zebraPlayer.error = msg;
}

// ------------------------------------------------------------------
// Loading

function blobBytes(id) {
  var b64 = $(id).textContent;
  return fetch('data:application/octet-stream;base64,' + b64)
    .then(function (r) { return r.arrayBuffer(); })
    .catch(function () {
      var s = atob(b64), u = new Uint8Array(s.length);
      for (var i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
      return u.buffer;
    })
    .then(function (b) { return new Uint8Array(b); });
}
function gunzip(u8) {
  var s = new Blob([u8]).stream().pipeThrough(new DecompressionStream('gzip'));
  return new Response(s).arrayBuffer().then(function (b) { return new Uint8Array(b); });
}
function put(u8) {
  if (!u8.length) return 0;
  var p = X.zw_alloc(u8.length);
  new Uint8Array(X.memory.buffer, p, u8.length).set(u8);
  return p;
}
function putStr(s) { var u = enc.encode(s); return [put(u), u.length]; }
function out() { return dec.decode(new Uint8Array(X.memory.buffer, X.zw_out_ptr(), X.zw_out_len()).slice()); }
// Links open only for http, https and mailto, as in the app (the player
// filters too; this guards the page on its own).
function safeLink(u) { return /^\s*(https?:\/\/|mailto:)/i.test(u); }

var media = {}; // asset key -> object URL or relative path

function load() {
  if (!window.WebAssembly || !window.DecompressionStream) { fail(L.unsupported); return; }
  var imports = { env: { zw_log: function (p, n) { console.error(dec.decode(new Uint8Array(X.memory.buffer, p, n).slice())); } } };
  blobBytes(M.wasm).then(gunzip)
    .then(function (w) { return WebAssembly.instantiate(w, imports); })
    .then(function (r) {
      X = r.instance.exports;
      X.zw_init();
      timings.wasm = performance.now() - started;
      return Promise.all(M.fonts.map(function (id) { return blobBytes(id).then(gunzip); }));
    })
    .then(function (fonts) {
      fonts.forEach(function (f) { X.zw_add_font(put(f), f.length); });
      return Promise.all(M.assets.map(function (a) {
        if (a.src) { media[a.key] = a.src; return null; }
        return blobBytes(a.blob).then(function (b) {
          if (a.kind === 'image') return [a, b];
          media[a.key] = URL.createObjectURL(new Blob([b], { type: a.mime }));
          return [a, new Uint8Array(0)];
        });
      }));
    })
    .then(function (assets) {
      assets.forEach(function (ab) {
        if (!ab) return;
        var a = ab[0], b = ab[1], k = putStr(a.key), e = putStr(a.ext);
        X.zw_add_asset(k[0], k[1], e[0], e[1], a.w, a.h, put(b), b.length);
      });
      // A video or audio file kept beside the page still needs its entry.
      M.assets.forEach(function (a) {
        if (!a.src) return;
        var k = putStr(a.key), e = putStr(a.ext);
        X.zw_add_asset(k[0], k[1], e[0], e[1], a.w, a.h, 0, 0);
      });
      return blobBytes(M.doc).then(gunzip);
    })
    .then(function (json) {
      if (X.zw_load(put(json), json.length) !== 0) { fail(out()); return; }
      timings.loaded = performance.now() - started;
      resize();
      new ResizeObserver(resize).observe(stage);
      bind();
      tick();
      timings.first_paint = performance.now() - started;
      window.zebraPlayer.ready = true;
    })
    .catch(function (e) { console.error(e); fail(L.unsupported); });
}

// ------------------------------------------------------------------
// Drawing

var ctx = canvas.getContext('2d');
var raf = 0, timer = 0, lastFrames = -1, state = null;
var frameTimes = [];
window.zebraPlayer.frameTimes = frameTimes;

function resize() {
  var r = stage.getBoundingClientRect();
  var dpr = Math.min(window.devicePixelRatio || 1, 2);
  X.zw_resize(r.width, r.height, dpr);
  wake();
}
function wake() {
  if (raf || !X) return;
  if (timer) { clearTimeout(timer); timer = 0; }
  raf = requestAnimationFrame(tick);
}
function tick() {
  raf = 0;
  var t0 = performance.now();
  var wait = X.zw_frame(t0 / 1000);
  var n = X.zw_frames();
  if (n !== lastFrames) {
    lastFrames = n;
    var w = X.zw_width(), h = X.zw_height();
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    if (w && h) {
      var px = new Uint8ClampedArray(X.memory.buffer, X.zw_pixels(), w * h * 4);
      ctx.putImageData(new ImageData(px, w, h), 0, 0);
    }
    frameTimes.push(performance.now() - t0);
    if (frameTimes.length > 600) frameTimes.shift();
  }
  sync();
  if (wait >= 0) {
    if (wait < 0.02) raf = requestAnimationFrame(tick);
    else timer = setTimeout(function () { timer = 0; wake(); }, wait * 1000);
  }
}

// ------------------------------------------------------------------
// State: control bar, blank screen, video and audio

var videos = {}, autoplayed = {}, lastStep = null, voice = null, music = null, gesture = false;

function sync() {
  X.zw_state();
  state = JSON.parse(out());
  window.zebraPlayer.state = state;
  var s = state;
  count.textContent = (s.step === null ? L.overview : L.step_of.replace('{step}', s.step + 1))
    .replace('{count}', s.count).replace(/[⁨⁩]/g, '');
  prevBtn.disabled = s.step === null && !s.off_path;
  nextBtn.disabled = s.at_end && !s.off_path && !s.blank;
  playBtn.hidden = !s.autoplay;
  setIcon(playBtn, s.autoplay_paused ? 'play' : 'pause', s.autoplay_paused ? L.play : L.pause);
  blank.className = s.blank || '';

  // A new step stops the last one's media and starts its voice-over.
  if (s.step !== lastStep) {
    lastStep = s.step;
    autoplayed = {};
    Object.keys(videos).forEach(function (id) { videos[id].pause(); });
    if (voice) { voice.pause(); voice = null; }
    if (s.voiceover && media[s.voiceover]) {
      voice = new Audio(media[s.voiceover]);
      voice.play().catch(function () {});
    }
  }
  // Videos on the step play once the camera arrives.
  s.autoplay_videos.forEach(function (id) {
    if (!autoplayed[id]) { autoplayed[id] = true; playVideo(id); }
  });
  // Lay the video elements over their place on the canvas.
  var shown = {};
  s.videos.forEach(function (b) {
    var v = videos[b.id];
    if (!v) return;
    shown[b.id] = true;
    v.style.width = b.w + 'px';
    v.style.height = b.h + 'px';
    v.style.transform = 'matrix(' + b.m.join(',') + ')';
    v.style.opacity = b.opacity;
  });
  Object.keys(videos).forEach(function (id) {
    var v = videos[id];
    var on = shown[id] && v.dataset.started === '1';
    v.classList.toggle('on', !!on);
  });
  duck();
}

function videoFor(id) {
  if (videos[id]) return videos[id];
  var info = M.videos[id];
  if (!info || !media[info.asset]) return null;
  var v = document.createElement('video');
  v.src = media[info.asset];
  v.playsInline = true;
  v.setAttribute('playsinline', '');
  v.preload = 'auto';
  v.muted = !!info.muted;
  if (info.alt) v.setAttribute('aria-label', info.alt);
  v.addEventListener('timeupdate', function () {
    if (info.end && v.currentTime >= info.end) {
      if (info.loop) v.currentTime = info.start || 0; else v.pause();
    }
  });
  v.addEventListener('ended', function () {
    if (info.loop) { v.currentTime = info.start || 0; v.play().catch(function () {}); }
  });
  v.addEventListener('play', playingChanged);
  v.addEventListener('pause', playingChanged);
  mediaLayer.appendChild(v);
  videos[id] = v;
  return v;
}
function playVideo(id) {
  var v = videoFor(id), info = M.videos[id];
  if (!v) return;
  var end = info.end || v.duration || Infinity;
  if (v.currentTime < (info.start || 0) || v.currentTime >= end - 0.05) v.currentTime = info.start || 0;
  v.dataset.started = '1';
  v.play().catch(function () {
    // No sound before a click (browser rule): try again muted.
    if (!v.muted) { v.muted = true; v.play().catch(function () {}); }
  });
  wake();
}
function toggleVideo(id) {
  var v = videoFor(id);
  if (!v) return;
  if (v.paused) playVideo(id); else v.pause();
}
function playingChanged() {
  var ids = Object.keys(videos).filter(function (id) { return !videos[id].paused; });
  var p = putStr(JSON.stringify(ids));
  X.zw_set_playing(p[0], p[1]);
  wake();
}
function duck() {
  if (!music) return;
  var loud = (voice && !voice.paused) || Object.keys(videos).some(function (id) { var v = videos[id]; return !v.paused && !v.muted; });
  music.volume = Math.max(0, Math.min(1, (M.soundtrack.volume || 0.6) * (loud ? 0.3 : 1)));
}
function startMusic() {
  if (gesture) return;
  gesture = true;
  if (M.soundtrack && media[M.soundtrack.asset] && !music) {
    music = new Audio(media[M.soundtrack.asset]);
    music.loop = M.soundtrack.loop !== false;
    duck();
    music.play().catch(function () {});
  }
  // Videos that had to start muted may have their sound now.
  Object.keys(videos).forEach(function (id) {
    var v = videos[id];
    if (!v.paused && v.muted && !M.videos[id].muted) v.muted = false;
  });
  if (voice && voice.paused) voice.play().catch(function () {});
}

// ------------------------------------------------------------------
// Control bar

var ICONS = {
  prev: '<path d="M10 3 5 8l5 5"/>',
  next: '<path d="M6 3l5 5-5 5"/>',
  full: '<path d="M2 6V2h4M10 2h4v4M14 10v4h-4M6 14H2v-4"/>',
  play: '<path d="M4 2.5v11l9-5.5z" fill="currentColor"/>',
  pause: '<path d="M5 3v10M11 3v10"/>'
};
function setIcon(b, name, label) {
  if (b.dataset.icon === name) return;
  b.dataset.icon = name;
  b.innerHTML = '<svg viewBox="0 0 16 16">' + ICONS[name] + '</svg>';
  b.title = label;
  b.setAttribute('aria-label', label);
}
function button(name, label, fn) {
  var b = document.createElement('button');
  b.type = 'button';
  setIcon(b, name, label);
  b.addEventListener('click', function (e) { e.stopPropagation(); startMusic(); fn(); wake(); });
  b.addEventListener('pointerdown', function (e) { e.stopPropagation(); });
  bar.appendChild(b);
  return b;
}
var bar = document.createElement('div');
bar.id = 'zw-bar';
bar.setAttribute('role', 'toolbar');
var prevBtn = button('prev', L.previous, function () { X.zw_command(1); });
var count = document.createElement('span');
count.id = 'zw-count';
count.setAttribute('aria-live', 'polite');
bar.appendChild(count);
var nextBtn = button('next', L.next, function () { X.zw_command(0); });
var playBtn = button('pause', L.pause, function () { X.zw_command(state && state.autoplay_paused ? 7 : 6); });
var fsOk = document.fullscreenEnabled || document.webkitFullscreenEnabled;
var fsBtn = button('full', L.full_screen, toggleFullScreen);
fsBtn.hidden = !fsOk;
document.body.appendChild(bar);

function toggleFullScreen() {
  var d = document;
  if (d.fullscreenElement || d.webkitFullscreenElement) (d.exitFullscreen || d.webkitExitFullscreen).call(d);
  else {
    var el = d.documentElement;
    var p = (el.requestFullscreen || el.webkitRequestFullscreen).call(el);
    if (p && p.catch) p.catch(function () {});
  }
}

// The bar hides after 2 s without the pointer moving (as in the app).
var idleTimer = 0;
function active() {
  bar.classList.remove('idle');
  stage.style.cursor = '';
  clearTimeout(idleTimer);
  idleTimer = setTimeout(function () {
    if (!bar.matches(':hover')) { bar.classList.add('idle'); stage.style.cursor = 'none'; }
  }, 2000);
}

// ------------------------------------------------------------------
// Input

function bind() {
  window.addEventListener('keydown', function (e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    var c = null;
    switch (e.key) {
      case 'ArrowRight': case 'ArrowDown': case ' ': case 'PageDown': case 'Enter': c = 0; break;
      case 'ArrowLeft': case 'ArrowUp': case 'PageUp': case 'Backspace': c = 1; break;
      case 'Home': case 'o': case 'O': c = 2; break;
      case 'End': c = 3; break;
      case 'b': case 'B': case '.': c = 4; break;
      case 'w': case 'W': c = 5; break;
      case 'f': case 'F': toggleFullScreen(); e.preventDefault(); return;
      default: return;
    }
    e.preventDefault();
    startMusic();
    X.zw_command(c);
    wake();
  });

  var pts = {}, moved = false, downAt = 0, start = null, pinch = null;
  function local(e) { var r = stage.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; }
  stage.addEventListener('pointerdown', function (e) {
    if (e.button === 2) return;
    try { stage.setPointerCapture(e.pointerId); } catch (_) {}
    pts[e.pointerId] = local(e);
    var ids = Object.keys(pts);
    if (ids.length === 1) { moved = false; downAt = performance.now(); start = pts[e.pointerId]; }
    if (ids.length === 2) {
      var a = pts[ids[0]], b = pts[ids[1]];
      pinch = { d: Math.hypot(a[0] - b[0], a[1] - b[1]), c: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2] };
      moved = true;
    }
    active();
  });
  stage.addEventListener('pointermove', function (e) {
    active();
    if (!pts[e.pointerId]) return;
    var p = local(e), q = pts[e.pointerId];
    pts[e.pointerId] = p;
    var ids = Object.keys(pts);
    if (ids.length === 2 && pinch) {
      var a = pts[ids[0]], b = pts[ids[1]];
      var d = Math.hypot(a[0] - b[0], a[1] - b[1]), c = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      X.zw_pan(c[0] - pinch.c[0], c[1] - pinch.c[1]);
      if (pinch.d > 0 && d > 0) X.zw_zoom(d / pinch.d, c[0], c[1]);
      pinch = { d: d, c: c };
      wake();
      return;
    }
    if (Math.hypot(p[0] - start[0], p[1] - start[1]) > 4) moved = true;
    // A finger swipes between steps; a mouse or pen drags the view.
    if (moved && e.pointerType !== 'touch') { X.zw_pan(p[0] - q[0], p[1] - q[1]); wake(); }
  });
  function up(e) {
    if (!pts[e.pointerId]) return;
    var p = pts[e.pointerId];
    delete pts[e.pointerId];
    var left = Object.keys(pts).length;
    if (left === 1) { pinch = null; return; }
    if (left > 0) return;
    pinch = null;
    if (e.type === 'pointercancel') return;
    startMusic();
    var dx = p[0] - start[0], dy = p[1] - start[1];
    if (e.pointerType === 'touch' && moved && performance.now() - downAt < 800 && Math.abs(dx) > 40 && Math.abs(dx) > Math.abs(dy) * 1.5) {
      X.zw_command(dx < 0 ? 0 : 1);
    } else if (!moved) {
      var r = X.zw_click(p[0], p[1]);
      if (r === 2) { var url = out(); if (safeLink(url)) window.open(url, '_blank', 'noopener'); }
      else if (r === 3) toggleVideo(out());
    }
    wake();
  }
  stage.addEventListener('pointerup', up);
  stage.addEventListener('pointercancel', up);
  stage.addEventListener('contextmenu', function (e) { e.preventDefault(); X.zw_command(1); wake(); });
  stage.addEventListener('wheel', function (e) {
    e.preventDefault();
    var p = local(e);
    var k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? 400 : 1;
    var dx = e.deltaX * k, dy = e.deltaY * k;
    // Pinch (ctrlKey) and mouse wheels zoom; trackpad scrolling pans.
    var wheel = e.deltaMode !== 0 || (e.deltaX === 0 && Number.isInteger(e.deltaY) && Math.abs(e.deltaY) >= 50);
    if (e.ctrlKey || e.metaKey || wheel) {
      var f = Math.min(2, Math.max(0.5, Math.exp(-dy * (e.ctrlKey ? 0.01 : 0.0035))));
      X.zw_zoom(f, p[0], p[1]);
    } else {
      X.zw_pan(-dx, -dy);
    }
    active();
    wake();
  }, { passive: false });
  document.addEventListener('fullscreenchange', resize);
  document.addEventListener('webkitfullscreenchange', resize);
  active();
}

load();
})();
