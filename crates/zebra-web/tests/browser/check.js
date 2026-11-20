// Browser check for File > Export > Web Page. Needs Node with Playwright
// (require('playwright')) and its Chromium.
//
//   cargo run -p zebra-web --example browser_fixture -- DIR [clip.mp4]
//   node crates/zebra-web/tests/browser/check.js DIR
//
// Opens the exported sample deck in headless Chromium, steps through it
// with the keyboard, and compares each step's canvas with the app
// renderer's capture of the same step (Renderer::capture). Then checks
// keyboard navigation, click-to-advance, the control bar, the video
// overlay, and the folder export. Screenshots go to DIR/shots.
// Exits non-zero on a failure.
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

const dir = process.argv[2];
const fx = JSON.parse(fs.readFileSync(path.join(dir, 'fixture.json'), 'utf8'));
fs.mkdirSync(path.join(dir, 'shots'), { recursive: true });
// A pixel differs when a channel is off by more than this; a step passes
// when under MAX_DIFF of its pixels differ.
const CHANNEL = 24, MAX_DIFF = 0.005;
let failures = 0;
function check(ok, what) { console.log((ok ? 'ok   ' : 'FAIL ') + what); if (!ok) failures++; }

async function settle(page) {
  await page.waitForFunction(() => zebraPlayer.state && zebraPlayer.state.arrived, null, { timeout: 15000 });
  await page.waitForTimeout(500); // fade-ins (0.3 s)
}
async function state(page) { return page.evaluate(() => zebraPlayer.state); }
async function key(page, k) { await page.keyboard.press(k); await page.waitForTimeout(50); await settle(page); }

async function diff(page, refFile) {
  const ref = 'data:image/png;base64,' + fs.readFileSync(refFile).toString('base64');
  return page.evaluate(async ([ref, CH]) => {
    const c = document.getElementById('zw-canvas');
    const a = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    const img = new Image();
    img.src = ref;
    await img.decode();
    const o = document.createElement('canvas');
    o.width = img.width; o.height = img.height;
    const x = o.getContext('2d');
    x.drawImage(img, 0, 0);
    const b = x.getImageData(0, 0, o.width, o.height).data;
    if (a.length !== b.length) return { size: [c.width, c.height, img.width, img.height] };
    let n = 0, sum = 0;
    for (let i = 0; i < a.length; i += 4) {
      const d = Math.max(Math.abs(a[i] - b[i]), Math.abs(a[i + 1] - b[i + 1]), Math.abs(a[i + 2] - b[i + 2]));
      sum += d;
      if (d > CH) n++;
    }
    return { frac: n / (a.length / 4), mean: sum / (a.length / 4) };
  }, [ref, CHANNEL]);
}

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: fx.width, height: fx.height }, deviceScaleFactor: 1 });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  const t0 = Date.now();
  await page.goto('file://' + fx.single);
  await page.waitForFunction(() => window.zebraPlayer && (zebraPlayer.ready || zebraPlayer.error), null, { timeout: 60000 });
  const timings = await page.evaluate(() => zebraPlayer.timings);
  console.log('loaded in', Date.now() - t0, 'ms; player timings', JSON.stringify(timings));
  check(!(await page.evaluate(() => zebraPlayer.error)), 'player starts');
  await settle(page);

  // Step through: each position against its capture.
  for (let i = 0; i < fx.sequence.length; i++) {
    const pos = fx.sequence[i];
    const s = await state(page);
    check(s.step === pos.step && s.build === pos.build, `at step ${pos.step} build ${pos.build} (got ${s.step}/${s.build})`);
    const d = await diff(page, path.join(dir, 'ref', pos.ref));
    const note = pos.overview ? ' (overview: the player dims later fade-ins, the capture hides them)' : '';
    if (d.size) check(false, `${pos.ref}: size ${d.size}`);
    else if (pos.overview) console.log(`info ${pos.ref}: ${(d.frac * 100).toFixed(3)}% pixels differ, mean ${d.mean.toFixed(3)}${note}`);
    else check(d.frac <= MAX_DIFF, `${pos.ref}: ${(d.frac * 100).toFixed(3)}% pixels differ (max ${MAX_DIFF * 100}%), mean ${d.mean.toFixed(3)}`);
    await page.screenshot({ path: path.join(dir, 'shots', pos.ref) });
    if (i + 1 < fx.sequence.length) await key(page, 'ArrowRight');
  }
  let s = await state(page);
  check(s.at_end, 'last step reached');
  if (fx.video) {
    const v = await page.evaluate(async () => {
      const v = document.querySelector('#zw-media video');
      if (!v) return null;
      await new Promise(r => setTimeout(r, 800));
      return { on: v.classList.contains('on'), t: v.currentTime, paused: v.paused, src: v.src.slice(0, 5), tf: v.style.transform };
    });
    check(v && v.on && !v.paused && v.t > 0, `video autoplays on its step over the canvas ${JSON.stringify(v)}`);
    await page.screenshot({ path: path.join(dir, 'shots', 'video.png') });
  }

  // Keyboard.
  await key(page, 'ArrowLeft');
  s = await state(page);
  check(s.step === fx.sequence[fx.sequence.length - 2].step, 'ArrowLeft goes back a step');
  await key(page, 'Home');
  s = await state(page);
  check(s.step === null, 'Home shows the overview');
  await key(page, 'End');
  s = await state(page);
  check(s.step === s.count - 1, 'End goes to the last step');
  await key(page, 'Home');
  await key(page, ' ');
  s = await state(page);
  check(s.step === 0, 'Space goes to the first step');
  await key(page, 'b');
  check((await state(page)).blank === 'black' && (await page.evaluate(() => document.getElementById('zw-blank').className)) === 'black', 'B blanks the screen');
  await key(page, 'b');

  // Control bar and clicks.
  await page.mouse.move(fx.width / 2, fx.height / 2);
  const bar = await page.evaluate(() => { const b = document.getElementById('zw-bar'); return { text: document.getElementById('zw-count').textContent, buttons: b.querySelectorAll('button:not([hidden])').length }; });
  check(bar.text === `1 of ${s.count}`, `control bar shows "${bar.text}"`);
  await page.screenshot({ path: path.join(dir, 'shots', 'control-bar.png') });
  await page.click('#zw-bar button:nth-of-type(2)'); // next
  await settle(page);
  check((await state(page)).step === 1, 'next button');
  await page.click('#zw-bar button:nth-of-type(1)'); // previous
  await settle(page);
  check((await state(page)).step === 0, 'previous button');
  // Empty space on the overview step goes to the next step.
  await page.mouse.click(4, 4);
  await settle(page);
  check((await state(page)).step === 1, 'click on empty space goes to the next step');
  // Wheel zoom leaves the path; the next key returns to it.
  await page.mouse.move(fx.width / 2, fx.height / 2);
  await page.mouse.wheel(0, -300);
  await page.waitForTimeout(100);
  check((await state(page)).off_path, 'wheel zooms off the path');
  await key(page, 'ArrowRight');
  check(!(await state(page)).off_path, 'next returns to the path');
  // Mouse drag pans.
  await page.mouse.move(600, 400);
  await page.mouse.down();
  await page.mouse.move(500, 380, { steps: 5 });
  await page.mouse.up();
  check((await state(page)).off_path, 'drag pans off the path');
  check(errors.length === 0, 'no page errors ' + JSON.stringify(errors));

  // Touch: a swipe left goes to the next step.
  const touch = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, hasTouch: true, isMobile: true });
  await touch.goto('file://' + fx.single);
  await touch.waitForFunction(() => window.zebraPlayer && zebraPlayer.ready, null, { timeout: 60000 });
  await settle(touch);
  const before = (await state(touch)).step;
  await touch.evaluate(() => {
    const st = document.getElementById('zw-stage');
    const ev = (type, x) => st.dispatchEvent(new PointerEvent(type, { pointerId: 7, pointerType: 'touch', clientX: x, clientY: 400, isPrimary: true, bubbles: true }));
    ev('pointerdown', 300); ev('pointermove', 250); ev('pointermove', 150); ev('pointerup', 120);
  });
  await settle(touch);
  check((await state(touch)).step === before + 1, 'swipe left goes to the next step (phone)');
  await touch.screenshot({ path: path.join(dir, 'shots', 'phone.png') });
  await touch.close();

  // Folder export: media beside the page.
  check(fx.folder_folder, 'folder export made a folder');
  const fp = await browser.newPage({ viewport: { width: fx.width, height: fx.height } });
  await fp.goto('file://' + fx.folder);
  await fp.waitForFunction(() => window.zebraPlayer && (zebraPlayer.ready || zebraPlayer.error), null, { timeout: 60000 });
  check(await fp.evaluate(() => zebraPlayer.ready), 'folder page starts');
  if (fx.video) {
    await fp.keyboard.press('End');
    await settle(fp);
    const v = await fp.evaluate(async () => { await new Promise(r => setTimeout(r, 800)); const v = document.querySelector('#zw-media video'); return v && { src: v.getAttribute('src'), t: v.currentTime }; });
    check(v && v.src.startsWith('media/') && v.t > 0, `folder video plays from ${v && v.src}`);
  }
  await browser.close();
  console.log(failures ? `${failures} FAILED` : 'all passed');
  process.exit(failures ? 1 : 0);
})();
