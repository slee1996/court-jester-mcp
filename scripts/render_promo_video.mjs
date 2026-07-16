#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';

const root = resolve(new URL('..', import.meta.url).pathname);
const out = resolve(process.argv[2] || join(root, 'site/assets/court-jester-promo.mp4'));
const rootCopy = resolve(join(root, 'court-jester-promo.mp4'));
const framesDir = resolve(join(root, 'tmp/promo-video-frames'));
const fps = Number(process.env.PROMO_FPS || 24);
const duration = Number(process.env.PROMO_DURATION || 24);
const width = 1920;
const height = 1080;
const totalFrames = Math.round(fps * duration);

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function html() {
  return String.raw`<!doctype html>
<html>
<head>
<meta charset="utf-8">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Instrument+Serif:ital@0;1&family=JetBrains+Mono:wght@400;500;700&family=Inter:wght@400;500;700;800&display=swap" rel="stylesheet">
<style>
:root {
  --bg: #08090c;
  --panel: #10141b;
  --panel2: #121a13;
  --ink: #f8f4e7;
  --dim: #a8a092;
  --muted: #65605a;
  --red: #ff4f65;
  --gold: #e0bd5e;
  --green: #92efa2;
  --blue: #7fdcff;
}
* { box-sizing: border-box; }
html, body { width: 1920px; height: 1080px; margin: 0; overflow: hidden; background: var(--bg); }
body { font-family: Inter, system-ui, sans-serif; color: var(--ink); }
.frame { position: relative; width: 1920px; height: 1080px; overflow: hidden; background:
  radial-gradient(circle at 12% 8%, rgba(224,189,94,.12), transparent 30%),
  radial-gradient(circle at 82% 28%, rgba(255,79,101,.10), transparent 28%),
  linear-gradient(180deg, #090a0d 0%, #07090c 100%); }
.noise { position:absolute; inset:0; opacity:.13; background-image: repeating-linear-gradient(0deg, rgba(255,255,255,.05) 0 1px, transparent 1px 3px); mix-blend-mode: overlay; }
.nav { position:absolute; left:72px; right:72px; top:44px; display:flex; justify-content:space-between; align-items:center; font-family:'JetBrains Mono', monospace; color:var(--dim); font-size:18px; letter-spacing:.04em; }
.logo { display:flex; gap:9px; align-items:flex-end; text-transform:uppercase; }
.logo strong { font-family:'Instrument Serif', serif; font-style:italic; font-size:34px; color:var(--ink); text-transform:none; font-weight:400; transform:translateY(5px); }
.nav .right { display:flex; gap:26px; }
.scene { position:absolute; inset:0; opacity:0; transform:translateY(16px); }
.kicker { font-family:'JetBrains Mono', monospace; color:var(--gold); font-size:20px; letter-spacing:.16em; text-transform:uppercase; margin-bottom:24px; }
h1, h2 { font-family:'Instrument Serif', serif; font-weight:400; line-height:.95; margin:0; letter-spacing:-.04em; }
h1 { font-size:122px; max-width:780px; }
h2 { font-size:92px; max-width:760px; }
em { font-style:italic; }
.copy { font-size:31px; line-height:1.35; color:#dfddd5; max-width:720px; margin-top:34px; }
.badge-row { display:flex; gap:18px; margin-top:34px; flex-wrap:wrap; }
.badge { font-family:'JetBrains Mono', monospace; border:1px solid rgba(224,189,94,.35); background:rgba(224,189,94,.08); color:#f3d57b; padding:12px 16px; font-size:17px; }
.terminal { position:absolute; right:110px; top:180px; width:820px; min-height:600px; border:1px solid rgba(248,244,231,.13); background:rgba(8,10,14,.88); box-shadow:0 30px 90px rgba(0,0,0,.52); padding:34px 38px; font-family:'JetBrains Mono', monospace; font-size:22px; line-height:1.58; }
.terminal::before { content:''; position:absolute; left:0; right:0; top:0; height:42px; border-bottom:1px solid rgba(248,244,231,.10); background:rgba(255,255,255,.02); }
.terminal-title { position:absolute; top:10px; left:22px; font-size:14px; color:var(--dim); }
.dot { display:inline-block; width:10px; height:10px; border-radius:50%; margin-right:8px; background:var(--gold); }
.line { opacity:0; transform:translateY(6px); white-space:pre-wrap; }
.prompt { color:var(--red); }
.ok { color:var(--green); }
.fail { color:var(--red); }
.meta { color:var(--dim); }
.mark { color:var(--red); text-decoration: underline; text-decoration-thickness:3px; text-underline-offset:5px; }
.left { position:absolute; left:150px; top:190px; width:820px; }
.center { position:absolute; left:120px; right:120px; top:150px; text-align:center; }
.chips { position:absolute; left:120px; right:120px; bottom:98px; display:flex; gap:18px; justify-content:center; flex-wrap:wrap; }
.chip { font-family:'JetBrains Mono', monospace; padding:16px 20px; border:1px solid rgba(255,79,101,.35); background:rgba(255,79,101,.08); color:#ffd5da; font-size:22px; transform:translateY(20px); opacity:0; }
.diff { position:absolute; left:980px; top:260px; width:760px; border:1px solid rgba(146,239,162,.22); background:rgba(10,17,12,.86); padding:34px 38px; font-family:'JetBrains Mono', monospace; font-size:24px; line-height:1.6; }
.diff .minus { color:#ff8391; }
.diff .plus { color:#92efa2; }
.diff .ctx { color:#b9b3a6; }
.arrow { position:absolute; left:860px; top:500px; width:130px; height:3px; background:var(--gold); transform-origin:left center; }
.arrow::after { content:''; position:absolute; right:-2px; top:-7px; border-left:16px solid var(--gold); border-top:8px solid transparent; border-bottom:8px solid transparent; }
.score-grid { position:absolute; left:150px; right:150px; top:350px; display:grid; grid-template-columns:repeat(4,1fr); gap:22px; }
.score { border:1px solid rgba(248,244,231,.14); background:rgba(14,18,24,.86); padding:28px; min-height:260px; position:relative; overflow:hidden; }
.score strong { display:block; font-family:'JetBrains Mono', monospace; font-size:52px; margin:26px 0 12px; }
.score span { font-family:'JetBrains Mono', monospace; color:var(--dim); text-transform:uppercase; letter-spacing:.08em; font-size:18px; }
.score p { color:var(--dim); font-size:19px; line-height:1.4; margin:0; }
.score.win { border-color:rgba(146,239,162,.55); box-shadow:0 0 0 1px rgba(146,239,162,.16) inset, 0 0 80px rgba(146,239,162,.10); }
.score.win strong { color:var(--green); }
.footer-cta { position:absolute; left:150px; right:150px; bottom:95px; display:flex; justify-content:space-between; align-items:flex-end; }
.cta-title { font-family:'Instrument Serif', serif; font-size:68px; line-height:.96; max-width:760px; }
.cta-code { font-family:'JetBrains Mono', monospace; color:var(--green); font-size:28px; border:1px solid rgba(146,239,162,.35); padding:22px 26px; background:rgba(146,239,162,.07); }
.flash { position:absolute; inset:-20px; background:radial-gradient(circle at 50% 50%, rgba(255,79,101,.24), transparent 36%); opacity:0; }
.underline { display:inline-block; position:relative; }
.underline::after { content:''; position:absolute; left:0; right:0; bottom:6px; height:5px; background:var(--red); transform:skewX(-18deg); z-index:-1; }
</style>
</head>
<body>
<div class="frame">
  <div class="noise"></div>
  <div class="flash" id="flash"></div>
  <div class="nav"><div class="logo">Court <strong>Jester</strong></div><div class="right"><span>verify</span><span>fuzz</span><span>repair</span></div></div>

  <section class="scene" id="s1">
    <div class="left">
      <div class="kicker">The usual agent loop</div>
      <h1>Looks done.<br><em>Isn’t.</em></h1>
      <p class="copy">The patch passes visible checks, the agent declares victory, and the hidden edge case waits patiently in production.</p>
      <div class="badge-row"><div class="badge">public tests pass</div><div class="badge">hidden edge untouched</div></div>
    </div>
    <div class="terminal" data-start="0.3">
      <div class="terminal-title"><span class="dot"></span>agent run</div>
      <div class="line" data-at="0.1"><span class="prompt">$</span> pytest tests/public</div>
      <div class="line ok" data-at="0.7">12 passed</div>
      <div class="line" data-at="1.2">agent: implementation complete ✅</div>
      <div class="line meta" data-at="1.8">hidden: nullish string leak still waiting</div>
    </div>
  </section>

  <section class="scene" id="s2">
    <div class="left">
      <div class="kicker">Court Jester enters</div>
      <h2>It asks the annoying questions.</h2>
      <p class="copy">Instead of another vague retry, the verifier throws adversarial inputs at the changed file until the patch contradicts itself.</p>
    </div>
    <div class="terminal" data-start="4.6">
      <div class="terminal-title"><span class="dot"></span>court-jester verify</div>
      <div class="line" data-at="0.1"><span class="prompt">$</span> court-jester verify --file query.py --language python</div>
      <div class="line ok" data-at="0.1">[parse]   passed    0 ms</div>
      <div class="line ok" data-at="1.0">[lint]    passed   48 ms</div>
      <div class="line fail" data-at="1.5">[execute] failed 31 ms</div>
      <div class="line meta" data-at="2.0">fuzz: 61 inputs · 58 passed · 3 crashed</div>
      <div class="line" data-at="2.5">repro: canonical_query({"flags": {"beta_checkout": None}})</div>
      <div class="line" data-at="3.0">property: nullish string leak</div>
      <div class="line mark" data-at="3.4">output: "flags=%7B%27beta_checkout%27%3A+None%7D"</div>
      <div class="line fail" data-at="4.0">verdict: fail</div>
    </div>
    <div class="chips">
      <div class="chip" data-at="0.3">None</div><div class="chip" data-at="0.6">""</div><div class="chip" data-at="0.9">[]</div><div class="chip" data-at="1.2">nested object</div><div class="chip" data-at="1.5">unicode key</div><div class="chip" data-at="1.8">cross-file contract</div>
    </div>
  </section>

  <section class="scene" id="s3">
    <div class="left">
      <div class="kicker">Repair signal</div>
      <h2>Not “try again.”<br><em>Fix this.</em></h2>
      <p class="copy">The agent gets the exact failing input and the bad output. That is the difference between search and repair.</p>
    </div>
    <div class="arrow"></div>
    <div class="diff" data-start="10.7">
      <div class="line ctx" data-at="0.1">function encodeValue(value) {</div>
      <div class="line minus" data-at="0.5">-  return String(value)</div>
      <div class="line plus" data-at="1.0">+  if (value == null) return undefined</div>
      <div class="line plus" data-at="1.4">+  if (Array.isArray(value)) return value.map(encodeValue)</div>
      <div class="line plus" data-at="1.8">+  return encodeURIComponent(String(value))</div>
      <div class="line ctx" data-at="2.2">}</div>
    </div>
  </section>

  <section class="scene" id="s4">
    <div class="left">
      <div class="kicker">Immediate rerun</div>
      <h2>The loop earns its exit.</h2>
      <p class="copy">Court Jester is not the final judge. It is the thing that refuses to let “looks plausible” be the stopping condition.</p>
    </div>
    <div class="terminal" data-start="15.2">
      <div class="terminal-title"><span class="dot" style="background:var(--green)"></span>court-jester verify</div>
      <div class="line" data-at="0.1"><span class="prompt">$</span> court-jester verify --file query.py --language python</div>
      <div class="line ok" data-at="0.6">[parse]   passed    0 ms</div>
      <div class="line ok" data-at="0.9">[lint]    passed   47 ms</div>
      <div class="line ok" data-at="1.3">[execute] passed   29 ms</div>
      <div class="line meta" data-at="1.8">fuzz: 58 inputs · 58 passed · 0 crashed</div>
      <div class="line ok" data-at="2.3">verdict: pass</div>
    </div>
  </section>

  <section class="scene" id="s5">
    <div class="center">
      <div class="kicker">Measured, not vibes</div>
      <h2 style="max-width:none">Concrete repros beat blind retries.</h2>
    </div>
    <div class="score-grid">
      <div class="score"><span>Baseline</span><strong>208/234</strong><p>one shot, no repair loop</p></div>
      <div class="score"><span>Blind retry</span><strong>216/234</strong><p>another attempt, no feedback</p></div>
      <div class="score"><span>Public repair</span><strong>205/234</strong><p>visible test feedback only</p></div>
      <div class="score win"><span>Verify repair</span><strong>230/234</strong><p>one extra shot with a concrete repro</p></div>
    </div>
    <div class="footer-cta">
      <div class="cta-title"><span class="underline">270/270</span> false-positive controls stayed clean.</div>
      <div class="cta-code">court-jester verify</div>
    </div>
  </section>
</div>
<script>
const scenes = [
  ['s1', 0.0, 4.8], ['s2', 4.2, 10.7], ['s3', 10.0, 15.2], ['s4', 14.5, 19.1], ['s5', 18.4, 24.0]
];
function clamp(x, a, b) { return Math.max(a, Math.min(b, x)); }
function smooth(a, b, x) { const v = clamp((x - a) / (b - a), 0, 1); return v * v * (3 - 2 * v); }
function opacity(t, start, end) { return smooth(start, start + .65, t) * (1 - smooth(end - .65, end, t)); }
function setBlockLines(block, t) {
  const start = Number(block.dataset.start || 0);
  block.querySelectorAll('.line').forEach((line) => {
    const at = start + Number(line.dataset.at || 0);
    const on = smooth(at, at + .28, t);
    line.style.opacity = on;
    line.style.transform = 'translateY(' + ((1 - on) * 7).toFixed(2) + 'px)';
  });
}
window.__setPromoTime = (t) => {
  for (const [id, start, end] of scenes) {
    const el = document.getElementById(id);
    const op = opacity(t, start, end);
    el.style.opacity = op;
    el.style.transform = 'translateY(' + ((1 - op) * 18).toFixed(2) + 'px)';
  }
  document.querySelectorAll('.terminal,.diff').forEach((block) => setBlockLines(block, t));
  document.querySelectorAll('.chip').forEach((chip) => {
    const at = 4.4 + Number(chip.dataset.at || 0);
    const on = smooth(at, at + .36, t) * (1 - smooth(9.8, 10.3, t));
    chip.style.opacity = on;
    chip.style.transform = 'translateY(' + ((1 - on) * 24).toFixed(2) + 'px) rotate(' + ((1 - on) * -4).toFixed(2) + 'deg)';
  });
  const flash = document.getElementById('flash');
  flash.style.opacity = Math.max(opacity(t, 6.2, 7.2), opacity(t, 17.8, 18.7)) * .9;
};
window.__setPromoTime(0);
</script>
</body>
</html>`;
}

rmSync(framesDir, { recursive: true, force: true });
mkdirSync(framesDir, { recursive: true });
mkdirSync(resolve(out, '..'), { recursive: true });

let chromium;
try {
  ({ chromium } = await import('playwright'));
} catch {
  console.error('Missing dependency: playwright. Run `npm install playwright --no-save --no-package-lock`, then rerun this script.');
  process.exit(1);
}

const browser = await chromium.launch({ headless: true, channel: process.env.PLAYWRIGHT_CHANNEL || 'chrome' });
const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
await page.setContent(html(), { waitUntil: 'networkidle' });

for (let i = 0; i < totalFrames; i++) {
  const t = i / fps;
  await page.evaluate((time) => window.__setPromoTime(time), t);
  await page.screenshot({ path: join(framesDir, `frame-${String(i).padStart(4, '0')}.png`), type: 'png' });
  if (i % fps === 0) console.log(`rendered ${Math.round(t)}s / ${duration}s`);
}
await browser.close();

run('ffmpeg', [
  '-y',
  '-framerate', String(fps),
  '-i', join(framesDir, 'frame-%04d.png'),
  '-vf', 'format=yuv420p',
  '-c:v', 'libx264',
  '-pix_fmt', 'yuv420p',
  '-movflags', '+faststart',
  out,
]);

if (out !== rootCopy) copyFileSync(out, rootCopy);
console.log(`Rendered ${out}`);
if (existsSync(rootCopy)) console.log(`Copied ${rootCopy}`);
