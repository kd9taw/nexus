// Shareable milestone cards (Journey v1.1 #1): render an achievement as a
// clean 1200×630 PNG (the standard social-card aspect) on an offscreen canvas
// and put it on the CLIPBOARD — paste into a club chat, X/Mastodon, email.
// Local-only: nothing is uploaded anywhere; the operator decides where it goes.
// Clipboard-image write needs WebView2/Chromium ClipboardItem; when that's
// unavailable we fall back to a plain PNG download.

import { pushToast } from '../toast'

export interface ShareCardData {
  /** The operator's callsign — the card's identity line. */
  call: string
  /** Big headline, e.g. "Level 12" or "DXCC Bronze — 25 entities". */
  headline: string
  /** Supporting line, e.g. "1,204 QSOs · 38 entities · 214 grids". */
  sub: string
  /** Small footer context, e.g. "Journey · Nexus". */
  footer: string
}

/** Render the card. Pure canvas drawing — exported for reuse/testing hooks. */
export function renderShareCard(d: ShareCardData): HTMLCanvasElement {
  const W = 1200
  const H = 630
  const canvas = document.createElement('canvas')
  canvas.width = W
  canvas.height = H
  const ctx = canvas.getContext('2d')
  if (!ctx) return canvas

  // Night-shack backdrop with a subtle radio-green wash.
  const bg = ctx.createLinearGradient(0, 0, W, H)
  bg.addColorStop(0, '#0b1220')
  bg.addColorStop(1, '#101c2e')
  ctx.fillStyle = bg
  ctx.fillRect(0, 0, W, H)
  const glow = ctx.createRadialGradient(W * 0.82, H * 0.2, 40, W * 0.82, H * 0.2, 520)
  glow.addColorStop(0, 'rgba(74, 222, 128, 0.16)')
  glow.addColorStop(1, 'rgba(74, 222, 128, 0)')
  ctx.fillStyle = glow
  ctx.fillRect(0, 0, W, H)

  // A faint band×hour "waterfall" strip along the bottom — the app's signature.
  for (let i = 0; i < 48; i++) {
    // Deterministic pseudo-noise (no Math.random — reproducible cards).
    const v = (Math.sin(i * 12.9898) * 43_758.5453) % 1
    ctx.fillStyle = `rgba(94, 234, 212, ${0.04 + Math.abs(v) * 0.1})`
    ctx.fillRect(24 + i * 24, H - 70 - Math.abs(v) * 60, 18, 60 + Math.abs(v) * 60)
  }

  ctx.textBaseline = 'top'
  // Callsign — mono, the ham identity.
  ctx.font = '600 44px "Cascadia Code", "JetBrains Mono", Consolas, monospace'
  ctx.fillStyle = '#5eead4'
  ctx.fillText(d.call.toUpperCase(), 64, 72)

  // Headline.
  ctx.font = '800 92px system-ui, "Segoe UI", sans-serif'
  ctx.fillStyle = '#f1f5f9'
  ctx.fillText(d.headline, 60, 190, W - 128)

  // Sub line.
  ctx.font = '400 40px system-ui, "Segoe UI", sans-serif'
  ctx.fillStyle = '#94a3b8'
  ctx.fillText(d.sub, 64, 320, W - 128)

  // Footer: context + date.
  ctx.font = '500 30px system-ui, "Segoe UI", sans-serif'
  ctx.fillStyle = '#64748b'
  const date = new Date().toISOString().slice(0, 10)
  ctx.fillText(`${d.footer} · ${date}`, 64, H - 120)
  return canvas
}

/** Render + copy to clipboard (fallback: download). Fire-and-forget with toasts. */
export function shareCard(d: ShareCardData): void {
  const canvas = renderShareCard(d)
  canvas.toBlob((blob) => {
    if (!blob) {
      pushToast('Could not render the share card', 'error')
      return
    }
    const clip = navigator.clipboard as Clipboard | undefined
    const CI = window.ClipboardItem
    if (clip?.write && CI) {
      clip
        .write([new CI({ 'image/png': blob })])
        .then(() => pushToast('Share card copied — paste it anywhere', 'success', 5000))
        .catch(() => downloadBlob(blob, d))
    } else {
      downloadBlob(blob, d)
    }
  }, 'image/png')
}

function downloadBlob(blob: Blob, d: ShareCardData): void {
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${d.call.toUpperCase()}-${d.headline.replace(/[^A-Za-z0-9]+/g, '-').toLowerCase()}.png`
  document.body.appendChild(a)
  a.click()
  a.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
  pushToast('Share card saved as a PNG download', 'success', 5000)
}
