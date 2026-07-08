// Capture an animated demo GIF of the Tempo UI from the in-browser mock.
//
// Drives headless Chrome over the built mock (ui/dist), grabs frames while the
// waterfall scrolls and decodes arrive, toggles the Fast<->Robust tier mid-clip,
// then assembles an optimized looping GIF with ffmpeg → docs/img/demo.gif.
//
//   npm --prefix ui run build        # build the mock first
//   node scripts/capture-gif.mjs
//
// Requires a local Chrome/Chromium (CHROME env overrides), puppeteer-core
// (scripts/ npm install), and ffmpeg on PATH.

import { createServer } from 'node:http'
import { readFile, mkdir, rm, readdir } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { extname, join, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import puppeteer from 'puppeteer-core'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '..')
const DIST = join(REPO, 'ui', 'dist')
const OUT = join(REPO, 'docs', 'img', 'demo.gif')
const FRAMES = '/tmp/tempo-gif-frames'
const PORT = 4181

const CHROME =
  process.env.CHROME ||
  ['/usr/bin/google-chrome-stable', '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser'].find(
    (p) => existsSync(p),
  )

const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon' }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const pad = (n) => String(n).padStart(4, '0')

function startServer() {
  const server = createServer(async (req, res) => {
    let p = decodeURIComponent((req.url || '/').split('?')[0])
    if (p === '/') p = '/index.html'
    let f = join(DIST, p)
    if (!existsSync(f)) f = join(DIST, 'index.html')
    res.writeHead(200, { 'Content-Type': MIME[extname(f)] || 'application/octet-stream' })
    res.end(await readFile(f))
  })
  return new Promise((ok) => server.listen(PORT, () => ok(server)))
}

// Click a top-bar control whose text contains all the given tokens.
async function clickByText(page, tokens) {
  return page.evaluate((toks) => {
    const els = [...document.querySelectorAll('button, [role="button"]')]
    const el = els.find((e) => {
      const t = (e.textContent || '').toLowerCase()
      return toks.every((k) => t.includes(k.toLowerCase()))
    })
    if (el) { el.click(); return true }
    return false
  }, tokens)
}

async function main() {
  if (!CHROME) throw new Error('No Chrome/Chromium found. Set CHROME=/path/to/chrome.')
  if (!existsSync(DIST)) throw new Error(`${DIST} missing — run: npm --prefix ui run build`)
  await rm(FRAMES, { recursive: true, force: true })
  await mkdir(FRAMES, { recursive: true })
  await mkdir(dirname(OUT), { recursive: true })

  const server = await startServer()
  const url = `http://localhost:${PORT}/`
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--hide-scrollbars', '--force-color-profile=srgb'],
    defaultViewport: { width: 1440, height: 900, deviceScaleFactor: 1 },
  })

  try {
    const page = await browser.newPage()
    await page.goto(url, { waitUntil: 'networkidle0' })
    await page.evaluate(() => {
      localStorage.setItem('tempo-theme', 'dark')
      localStorage.setItem('tempo-onboarded', '1')
      localStorage.setItem('tempo-demo-dismissed', '1')
    })
    await page.reload({ waitUntil: 'networkidle0' })
    await page.waitForSelector('.app:not(.loading)', { timeout: 15000 })
    await sleep(2500) // let the waterfall/decodes seed

    const TOTAL = 48
    const TO_ROBUST = 16
    const TO_FAST = 34
    for (let i = 1; i <= TOTAL; i++) {
      if (i === TO_ROBUST) await clickByText(page, ['robust', 'dx1'])
      if (i === TO_FAST) await clickByText(page, ['fast', 'ft1'])
      await page.screenshot({ path: join(FRAMES, `frame-${pad(i)}.png`) })
      await sleep(120)
    }
    console.log(`captured ${(await readdir(FRAMES)).length} frames`)
  } finally {
    await browser.close()
    server.close()
  }

  console.log('assembling GIF with ffmpeg…')
  execFileSync(
    'ffmpeg',
    [
      '-y', '-framerate', '8', '-i', join(FRAMES, 'frame-%04d.png'),
      '-vf', 'scale=960:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3',
      '-loop', '0', OUT,
    ],
    { stdio: 'inherit' },
  )
  await rm(FRAMES, { recursive: true, force: true })
  console.log('wrote', OUT)
}

main().catch((e) => { console.error(e); process.exit(1) })
