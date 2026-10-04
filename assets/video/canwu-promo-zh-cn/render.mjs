// Renders scenes.html frame by frame with headless Chromium, then muxes the
// frames with the generated soundtrack into an H.264/AAC MP4 and writes an SRT.
//
// Usage: node render.mjs [--workers N] [--from SECONDS] [--to SECONDS]
//                        [--stills t1,t2,...] [--skip-frames] [--covers]
// Requires: Node 18+, the `playwright` package, ffmpeg, and build/music.wav
// (python3 music.py). Run python3 fetch_fonts.py once first.

import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const require = createRequire(import.meta.url);
let chromium;
try {
  ({ chromium } = require('playwright'));
} catch {
  // Fall back to a globally installed playwright.
  const globalRoot = spawnSync('npm', ['root', '-g'], { encoding: 'utf8', shell: process.platform === 'win32' }).stdout.trim();
  ({ chromium } = require(join(globalRoot, 'playwright')));
}

const here = dirname(fileURLToPath(import.meta.url));
const build = join(here, 'build');
const frames = join(build, 'frames');
const timeline = JSON.parse(readFileSync(join(here, 'timeline.json'), 'utf8'));
const args = process.argv.slice(2);
const opt = (name, fallback) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : fallback; };
const workers = parseInt(opt('--workers', '4'), 10);
const fps = timeline.fps;
const total = Math.round(timeline.duration * fps);
const fromFrame = Math.round(parseFloat(opt('--from', '0')) * fps);
const toFrame = Math.min(total, Math.round(parseFloat(opt('--to', String(timeline.duration))) * fps));
const stills = opt('--stills', null);

function srtTime(sec) {
  const ms = Math.round(sec * 1000);
  const h = Math.floor(ms / 3600000), m = Math.floor(ms / 60000) % 60, s = Math.floor(ms / 1000) % 60, r = ms % 1000;
  const pad = (n, w = 2) => String(n).padStart(w, '0');
  return `${pad(h)}:${pad(m)}:${pad(s)},${pad(r, 3)}`;
}

async function openPage(browser) {
  const page = await browser.newPage({ viewport: { width: timeline.width, height: timeline.height }, deviceScaleFactor: 1 });
  page.on('pageerror', e => console.error('page error:', e.message));
  await page.addInitScript(`window.TIMELINE = ${JSON.stringify(timeline)};`);
  await page.goto(pathToFileURL(join(here, 'scenes.html')).href);
  await page.evaluate(() => window.ready);
  return page;
}

async function renderRange(browser, start, end, worker) {
  const page = await openPage(browser);
  for (let f = start; f < end; f++) {
    await page.evaluate(t => window.renderAt(t), f / fps);
    await page.screenshot({ path: join(frames, `f${String(f).padStart(5, '0')}.jpg`), type: 'jpeg', quality: 94 });
    if ((f - start) % 150 === 0) console.log(`worker ${worker}: frame ${f}/${end}`);
  }
  await page.close();
}

function writeSrt() {
  const body = timeline.subtitles.map((c, i) => `${i + 1}\n${srtTime(c.start)} --> ${srtTime(c.end)}\n${c.text}\n`).join('\n');
  writeFileSync(join(build, 'canwu-promo-zh-cn.srt'), body, 'utf8');
}

function run(cmd, cmdArgs) {
  console.log(`$ ${cmd} ${cmdArgs.join(' ')}`);
  const r = spawnSync(cmd, cmdArgs, { stdio: 'inherit' });
  if (r.status !== 0) throw new Error(`${cmd} exited with ${r.status}`);
}

const browser = await chromium.launch({ args: ['--allow-file-access-from-files', '--font-render-hinting=none'] });
mkdirSync(frames, { recursive: true });

if (stills) {
  // Quick inspection: write PNG stills at the given timestamps.
  const page = await openPage(browser);
  for (const t of stills.split(',').map(Number)) {
    await page.evaluate(tt => window.renderAt(tt), t);
    await page.screenshot({ path: join(build, `still-${t.toFixed(2)}.png`) });
    console.log(`still ${t}`);
  }
  await browser.close();
  process.exit(0);
}

if (args.includes('--covers')) {
  // Bilibili covers: 16:9 for desktop placements, 4:3 for the mobile feed.
  for (const [ratio, width] of [['16x9', 1920], ['4x3', 1440]]) {
    const page = await browser.newPage({ viewport: { width, height: 1080 }, deviceScaleFactor: 1 });
    page.on('pageerror', e => console.error('page error:', e.message));
    await page.goto(`${pathToFileURL(join(here, 'cover.html')).href}?ratio=${ratio}`);
    await page.evaluate(() => window.ready);
    const path = join(build, `canwu-cover-${ratio}.jpg`);
    await page.screenshot({ path, type: 'jpeg', quality: 92 });
    console.log(`wrote ${path}`);
    await page.close();
  }
  await browser.close();
  process.exit(0);
}

if (!args.includes('--skip-frames')) {
  const span = toFrame - fromFrame, per = Math.ceil(span / workers);
  const jobs = [];
  for (let w = 0; w < workers; w++) {
    const a = fromFrame + w * per, b = Math.min(toFrame, a + per);
    if (a < b) jobs.push(renderRange(browser, a, b, w));
  }
  await Promise.all(jobs);
}
await browser.close();

writeSrt();
const music = join(build, 'music.wav');
if (!existsSync(music)) throw new Error('build/music.wav is missing; run python3 music.py first');
const out = join(build, 'canwu-promo-zh-cn.mp4');
if (existsSync(out)) rmSync(out);
run('ffmpeg', ['-hide_banner', '-loglevel', 'warning', '-y',
  '-framerate', String(fps), '-i', join(frames, 'f%05d.jpg'),
  '-i', music,
  '-map', '0:v', '-map', '1:a',
  '-c:v', 'libx264', '-preset', 'slow', '-crf', '18', '-tune', 'animation', '-pix_fmt', 'yuv420p',
  '-c:a', 'aac', '-b:a', '192k',
  '-t', String(timeline.duration), '-movflags', '+faststart',
  '-metadata', `title=${timeline.title}`, '-metadata:s:a:0', 'language=chi',
  out]);
console.log(`wrote ${out}`);
