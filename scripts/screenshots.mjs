// Auto-capture Tempo UI screenshots from the in-browser mock — no rig needed.
//
// Builds nothing itself: run `npm --prefix ui run build` first, then this drives
// a headless Chrome over the built mock (ui/dist) and writes themed PNGs to
// docs/img/. The mock is the same standalone engine that powers the live demo,
// so these are real renders of the actual UI, just with simulated data.
//
//   node scripts/screenshots.mjs
//
// Requires a local Chrome/Chromium (CHROME env overrides the auto-detected path)
// and `npm install` in scripts/ (puppeteer-core).

import { createServer } from 'node:http'
import { readFile, mkdir } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { extname, join, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import puppeteer from 'puppeteer-core'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '..')
const DIST = join(REPO, 'ui', 'dist')
const OUT = join(REPO, 'docs', 'img')
const PORT = 4178

const CHROME =
  process.env.CHROME ||
  ['/usr/bin/google-chrome-stable', '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser']
    .find((p) => existsSync(p))

const MIME = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
  '.json': 'application/json',
  '.woff2': 'font/woff2',
}

// Themes drive the hero / showcase shots.
const THEMES = ['dark', 'light', 'amber']
// Per-view shots (captured on the dark theme) — label matches the ModeNav button text.
const VIEWS = [
  { file: 'view-qso', label: 'QSO' },
  { file: 'view-fieldday', label: 'Field Day' },
  { file: 'view-band', label: 'Band' },
  { file: 'view-logbook', label: 'Logbook' },
  { file: 'view-settings', label: 'Settings' },
]

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

function startServer() {
  const server = createServer(async (req, res) => {
    try {
      let urlPath = decodeURIComponent((req.url || '/').split('?')[0])
      if (urlPath === '/' || urlPath === '') urlPath = '/index.html'
      let filePath = join(DIST, urlPath)
      if (!existsSync(filePath)) filePath = join(DIST, 'index.html') // SPA fallback
      const body = await readFile(filePath)
      res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] || 'application/octet-stream' })
      res.end(body)
    } catch (e) {
      res.writeHead(500)
      res.end(String(e))
    }
  })
  return new Promise((ok) => server.listen(PORT, () => ok(server)))
}

async function settle(page) {
  await page.waitForSelector('.app:not(.loading)', { timeout: 15000 })
  // Let the mock seed stations/decodes and the waterfall accumulate rows.
  await sleep(3000)
}

async function main() {
  if (!CHROME) throw new Error('No Chrome/Chromium found. Set CHROME=/path/to/chrome.')
  if (!existsSync(DIST)) throw new Error(`${DIST} missing — run: npm --prefix ui run build`)
  await mkdir(OUT, { recursive: true })

  const server = await startServer()
  const url = `http://localhost:${PORT}/`
  console.log(`serving ${DIST} → ${url} (chrome: ${CHROME})`)

  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: 'new',
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--hide-scrollbars', '--force-color-profile=srgb'],
    defaultViewport: { width: 1440, height: 900, deviceScaleFactor: 2 },
  })

  try {
    const page = await browser.newPage()
    page.on('console', (m) => m.type() === 'error' && console.log('  [page error]', m.text()))

    // Hero / theme showcase — default Chat view in each theme.
    for (const theme of THEMES) {
      await page.goto(url, { waitUntil: 'networkidle0' })
      await page.evaluate((t) => {
        localStorage.setItem('tempo-theme', t)
        localStorage.setItem('tempo-onboarded', '1')
        localStorage.setItem('tempo-demo-dismissed', '1')
      }, theme)
      await page.reload({ waitUntil: 'networkidle0' })
      await settle(page)
      const file = join(OUT, `app-${theme}.png`)
      await page.screenshot({ path: file })
      console.log('captured', file)
    }

    // Per-view shots on the dark theme.
    await page.goto(url, { waitUntil: 'networkidle0' })
    await page.evaluate(() => {
      localStorage.setItem('tempo-theme', 'dark')
      localStorage.setItem('tempo-onboarded', '1')
    })
    await page.reload({ waitUntil: 'networkidle0' })
    await settle(page)
    for (const v of VIEWS) {
      const clicked = await page.evaluate((label) => {
        const btn = [...document.querySelectorAll('button.mode-btn')].find((b) =>
          (b.textContent || '').trim().toLowerCase().includes(label.toLowerCase()),
        )
        if (btn) {
          btn.click()
          return true
        }
        return false
      }, v.label)
      if (!clicked) {
        console.log('  (skip)', v.label, '— nav button not found')
        continue
      }
      await sleep(1800)
      const file = join(OUT, `${v.file}.png`)
      await page.screenshot({ path: file })
      console.log('captured', file)
    }
  } finally {
    await browser.close()
    server.close()
  }
  console.log('done.')
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
