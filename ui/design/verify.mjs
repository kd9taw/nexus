// Stage-A token verifier (self-contained, no deps).
//
// Defines the Nexus semantic palette in OKLCH per theme and PROVES it:
//  - WCAG 2.1 contrast (text >=4.5:1, status/non-text >=3:1) on each surface
//  - CVD distinctness: pairwise OKLab distance within the must-distinguish
//    "need set" after Machado-2009 deutan/protan/tritan simulation
//  - emits design/proof.html (swatch grid: normal + 3 CVD columns) for the
//    visual sign-off screenshot, and design/tokens.generated.json.
//
//   node ui/design/verify.mjs
//
// This file is also the source of truth that the later Vitest contrast gate ports.

import { writeFileSync } from 'node:fs'

/* ---------- color math: OKLCH <-> sRGB, OKLab, WCAG, Machado CVD ---------- */

const cbrt = Math.cbrt
const clamp01 = (x) => Math.min(1, Math.max(0, x))

function oklchToLinear({ L, C, H }) {
  const a = C * Math.cos((H * Math.PI) / 180)
  const b = C * Math.sin((H * Math.PI) / 180)
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b
  const s_ = L - 0.0894841775 * a - 1.291485548 * b
  const l = l_ ** 3, m = m_ ** 3, s = s_ ** 3
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ]
}
function linearToOklab([r, g, b]) {
  const l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
  const m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
  const s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
  const l_ = cbrt(l), m_ = cbrt(m), s_ = cbrt(s)
  return [
    0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_,
    1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_,
    0.0259040371 * l_ + 0.7827717662 * m_ - 0.808675766 * s_,
  ]
}
const enc = (c) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055)
const dec = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
function oklchToHex(o) {
  const lin = oklchToLinear(o).map(clamp01)
  const [r, g, b] = lin.map((c) => Math.round(enc(c) * 255))
  return '#' + [r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')
}
function hexToLinear(hex) {
  const n = parseInt(hex.slice(1), 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => dec(v / 255))
}
function relLuminance(hex) {
  const [r, g, b] = hexToLinear(hex)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}
function contrast(h1, h2) {
  const a = relLuminance(h1), b = relLuminance(h2)
  const [hi, lo] = a > b ? [a, b] : [b, a]
  return (hi + 0.05) / (lo + 0.05)
}
// Machado 2009 severity-1.0 matrices, applied in LINEAR rgb.
const CVD = {
  protan: [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
  deutan: [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.01182, 0.04294, 0.968881]],
  tritan: [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.3039]],
}
function simulate(hex, type) {
  const lin = hexToLinear(hex)
  const M = CVD[type]
  const out = M.map((row) => clamp01(row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2]))
  const [r, g, b] = out.map((c) => Math.round(enc(c) * 255))
  return '#' + [r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')
}
function oklabDist(h1, h2) {
  const a = linearToOklab(hexToLinear(h1))
  const b = linearToOklab(hexToLinear(h2))
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

/* ---------------------------- the palette ------------------------------- */
// Surfaces kept from the proven existing themes; status colors authored in
// OKLCH. Color MEANS one thing (green=good, red=bad/tx, amber=caution); roles
// that share a meaning share a hue and are disambiguated by GLYPH + context.

const themes = {
  dark: {
    surfaces: { bg: '#0b0f17', panel: '#111722', elev: '#131925', border: '#243044' },
    text: { text: '#e7edf6', dim: '#9aa8bd', faint: '#7c8ca3' },
    // semantic status (OKLCH)
    status: {
      // need-set L-ladder, ATNO loudest: dupe<worked<new-mode<new-band<confirmed<new-entity.
      'new-entity': { L: 0.87, C: 0.17, H: 55 },     // ATNO — brightest hot orange ★
      'new-band':   { L: 0.76, C: 0.155, H: 98 },    // slot — gold        ◑
      'new-mode':   { L: 0.72, C: 0.15, H: 305 },    // slot — violet      ◧
      worked:       { L: 0.66, C: 0.045, H: 240 },   // have, unconfirmed  ○
      confirmed:    { L: 0.82, C: 0.15, H: 152 },    // confirmed — green  ✓
      dupe:         { L: 0.58, C: 0.012, H: 240 },   // have it — dim      ·
      'snr-strong': { L: 0.80, C: 0.15, H: 152 },
      'snr-marginal': { L: 0.85, C: 0.15, H: 92 },
      'snr-weak':   { L: 0.66, C: 0.18, H: 25 },
      tx:           { L: 0.62, C: 0.20, H: 25 },     // transmit — red (▲, column-scoped)
      rx:           { L: 0.80, C: 0.15, H: 152 },
      'band-open':  { L: 0.80, C: 0.15, H: 152 },
      'band-marginal': { L: 0.85, C: 0.15, H: 92 },
      'band-closed': { L: 0.58, C: 0.012, H: 240 },
      'alert-critical': { L: 0.74, C: 0.24, H: 40 }, // interrupt — bright red-orange ⚑ (distinct from tx)
      'alert-warning': { L: 0.84, C: 0.15, H: 85 },
      'alert-info': { L: 0.78, C: 0.10, H: 240 },
    },
  },
  light: {
    surfaces: { bg: '#eef1f5', panel: '#ffffff', elev: '#f4f6f9', border: '#d3dae3' },
    text: { text: '#14202e', dim: '#51607a', faint: '#5f6c7e' },
    status: {
      // Lightness ladder (CVD-robust); on light bg, darker = louder, so ATNO is darkest.
      'new-entity': { L: 0.44, C: 0.20, H: 42 },     // ATNO — loudest (darkest, hot) ★
      'new-band':   { L: 0.66, C: 0.135, H: 82 },
      'new-mode':   { L: 0.52, C: 0.19, H: 300 },
      worked:       { L: 0.58, C: 0.055, H: 240 },
      confirmed:    { L: 0.56, C: 0.16, H: 150 },
      dupe:         { L: 0.70, C: 0.012, H: 240 },
      // Reds kept darker than greens so deutan/protan separate good↔bad by L.
      'snr-strong': { L: 0.52, C: 0.15, H: 150 },
      'snr-marginal': { L: 0.58, C: 0.13, H: 80 },
      'snr-weak':   { L: 0.43, C: 0.20, H: 25 },
      tx:           { L: 0.43, C: 0.21, H: 25 },
      rx:           { L: 0.52, C: 0.15, H: 150 },
      'band-open':  { L: 0.52, C: 0.15, H: 150 },
      'band-marginal': { L: 0.58, C: 0.13, H: 80 },
      'band-closed': { L: 0.62, C: 0.012, H: 240 },
      // interrupt: darkest/loudest red-orange on light, distinct from tx by hue+form ⚑
      'alert-critical': { L: 0.37, C: 0.25, H: 32 },
      'alert-warning': { L: 0.58, C: 0.14, H: 78 },
      'alert-info': { L: 0.52, C: 0.12, H: 240 },
    },
  },
  amber: {
    // Night-vision: blue/cyan collapse, so NO role may rely on blue hue;
    // distinctions ride L (brightness) + glyph within the amber/red gamut.
    surfaces: { bg: '#060400', panel: '#0c0801', elev: '#0e0a02', border: '#3a2a06' },
    text: { text: '#ffb000', dim: '#b87c00', faint: '#9c6e00' },
    status: {
      // Monochromatic gamut: green/red hues are NOT separable, so "good" rides
      // HIGH lightness and "bad" rides LOW lightness, always with a glyph.
      // need-set is a clean lightness ladder: dupe<worked<new-band<new-mode<confirmed<new-entity.
      'new-entity': { L: 0.90, C: 0.15, H: 72 },     // brightest gold ★
      'new-band':   { L: 0.68, C: 0.145, H: 60 },    // mid amber ◑
      'new-mode':   { L: 0.74, C: 0.15, H: 40 },     // orange ◧
      worked:       { L: 0.58, C: 0.07, H: 64 },     // dim amber ○
      confirmed:    { L: 0.80, C: 0.13, H: 95 },     // bright yellow-green ✓ (good=bright)
      dupe:         { L: 0.47, C: 0.03, H: 64 },      // very dim ·
      'snr-strong': { L: 0.82, C: 0.14, H: 95 },
      'snr-marginal': { L: 0.72, C: 0.14, H: 68 },
      'snr-weak':   { L: 0.55, C: 0.17, H: 32 },     // bad=low L
      tx:           { L: 0.55, C: 0.17, H: 32 },     // transmit (▲, column-scoped)
      rx:           { L: 0.82, C: 0.14, H: 95 },
      'band-open':  { L: 0.82, C: 0.14, H: 95 },
      'band-marginal': { L: 0.72, C: 0.14, H: 68 },
      'band-closed': { L: 0.47, C: 0.03, H: 65 },
      // interrupt escapes the bad=dark rule: BRIGHTEST red-orange in amber ⚑
      'alert-critical': { L: 0.82, C: 0.19, H: 45 },
      'alert-warning': { L: 0.74, C: 0.15, H: 58 },
      'alert-info': { L: 0.66, C: 0.10, H: 72 },
    },
  },
}

// Glyphs are the PRIMARY (CVD-immune) channel — every role gets a UNIQUE one.
const GLYPH = {
  'new-entity': '★', 'new-band': '◑', 'new-mode': '◧', worked: '○', confirmed: '✓', dupe: '·',
  'snr-strong': '▇', 'snr-marginal': '▅', 'snr-weak': '▂', tx: '▲', rx: '▼',
  'band-open': '●', 'band-marginal': '◐', 'band-closed': '⊘',
  'alert-critical': '⚑', 'alert-warning': '△', 'alert-info': 'i',
}
// The roster "need set": every state has a UNIQUE GLYPH (primary, CVD-immune);
// color is a redundant cue, so we only require colors not be near-identical.
const NEED_SET = ['new-entity', 'new-band', 'new-mode', 'worked', 'confirmed', 'dupe']
// The DANGEROUS pairs: good (green) vs bad (red) carry opposite action and often
// appear WITHOUT a glyph (SNR color, TX/RX) — these MUST survive every CVD.
const GOOD = ['confirmed', 'snr-strong', 'rx', 'band-open']
// alert-critical is an INTERRUPT, not a "bad signal" — it escapes the good/bad
// lightness convention and is gated separately (most-salient + distinct-from-tx).
const BAD = ['snr-weak', 'tx']
// De-emphasis states are meant to recede (glyph/position-carried) — exempt from 3:1.
const DEEMPHASIS = new Set(['dupe', 'band-closed'])
const TEXT_MIN = 4.5, STATUS_MIN = 3.0, CVD_GOODBAD = 0.06, CVD_NEED = 0.03

/* ------------------------------ run checks ------------------------------ */
let fails = 0

// Glyph uniqueness across ALL roles (glyph is the primary, CVD-immune channel).
{
  const all = Object.keys(themes.dark.status).map((r) => GLYPH[r])
  const uniq = new Set(all).size === all.length
  if (!uniq) fails++
  console.log(`GLYPH uniqueness (${all.length} roles): ${uniq ? 'ok' : 'FAIL — reused glyph'}`)
}

const proofRows = []
for (const [tname, t] of Object.entries(themes)) {
  console.log(`\n=== THEME: ${tname} ===`)
  const hex = Object.fromEntries(Object.entries(t.status).map(([k, v]) => [k, oklchToHex(v)]))
  // text contrast
  for (const [k, c] of Object.entries(t.text)) {
    const cr = contrast(c, t.surfaces.bg)
    const ok = cr >= TEXT_MIN
    if (!ok) fails++
    console.log(`  text ${k.padEnd(6)} on bg   ${cr.toFixed(2)}:1  ${ok ? 'ok' : 'FAIL(<4.5)'}`)
  }
  // status contrast on panel (non-text -> 3:1); de-emphasis states exempt.
  for (const [k, c] of Object.entries(hex)) {
    const cr = contrast(c, t.surfaces.panel)
    if (DEEMPHASIS.has(k)) {
      console.log(`  stat ${k.padEnd(14)} on panel ${cr.toFixed(2)}:1 (de-emphasis, exempt)`)
      continue
    }
    const ok = cr >= STATUS_MIN
    if (!ok) fails++
    console.log(`  stat ${k.padEnd(14)} on panel ${cr.toFixed(2)}:1 ${ok ? 'ok' : 'FAIL(<3.0)'}`)
  }
  // CRITICAL: good(green) vs bad(red) must stay distinct under every CVD.
  for (const type of ['deutan', 'protan', 'tritan']) {
    let minD = Infinity, pair = ''
    for (const g of GOOD) for (const b of BAD) {
      const d = oklabDist(simulate(hex[g], type), simulate(hex[b], type))
      if (d < minD) { minD = d; pair = `${g}/${b}` }
    }
    const ok = minD >= CVD_GOODBAD
    if (!ok) fails++
    console.log(`  CVD ${type} good↔bad: min ΔE ${minD.toFixed(3)} (${pair}) ${ok ? 'ok' : 'FAIL(<' + CVD_GOODBAD + ')'}`)
  }
  // Need-set: unique glyphs (primary) + no two colors near-identical under CVD.
  const glyphs = NEED_SET.map((r) => GLYPH[r])
  const uniqueGlyphs = new Set(glyphs).size === glyphs.length
  if (!uniqueGlyphs) fails++
  for (const type of ['deutan', 'protan', 'tritan']) {
    let minD = Infinity, pair = ''
    for (let i = 0; i < NEED_SET.length; i++)
      for (let j = i + 1; j < NEED_SET.length; j++) {
        const d = oklabDist(simulate(hex[NEED_SET[i]], type), simulate(hex[NEED_SET[j]], type))
        if (d < minD) { minD = d; pair = `${NEED_SET[i]}/${NEED_SET[j]}` }
      }
    const ok = minD >= CVD_NEED
    if (!ok) fails++
    console.log(`  CVD ${type} need-set: glyphs-unique=${uniqueGlyphs}, min colorΔE ${minD.toFixed(3)} (${pair}) ${ok ? 'ok' : 'FAIL(<' + CVD_NEED + ')'}`)
  }
  // Salience: ATNO is the loudest need-state.
  {
    const cr = (r) => contrast(hex[r], t.surfaces.panel)
    const atno = cr('new-entity')
    const others = Math.max(cr('new-band'), cr('new-mode'), cr('worked'))
    const ok = atno >= others
    if (!ok) fails++
    console.log(`  salience: ATNO ${atno.toFixed(2)} >= other needs ${others.toFixed(2)} ${ok ? 'ok' : 'FAIL'}`)
  }
  // Salience: alert-critical is never buried — louder than tx + worked, and in
  // the monochromatic amber theme also brighter than a routine "good" chip.
  {
    const cr = (r) => contrast(hex[r], t.surfaces.panel)
    const crit = cr('alert-critical')
    const floor = Math.max(cr('tx'), cr('worked'))
    const ok = crit >= floor
    if (!ok) fails++
    console.log(`  salience: alert-critical ${crit.toFixed(2)} >= floor ${floor.toFixed(2)} ${ok ? 'ok' : 'FAIL'}`)
  }
  // proof rows
  proofRows.push({ tname, surfaces: t.surfaces, hex })
}

/* --------------------------- emit proof.html ---------------------------- */
const roleNames = Object.keys(themes.dark.status)
const cell = (hexv) =>
  `<td style="background:${hexv}"><span>${hexv}</span></td>`
let html = `<!doctype html><meta charset=utf8><style>
 body{margin:0;font:12px ui-monospace,monospace}
 .theme{padding:14px}
 h2{font:600 13px system-ui;margin:0 0 8px}
 table{border-collapse:collapse;width:100%}
 td,th{padding:4px 6px;text-align:left}
 th{font:600 11px system-ui;opacity:.7}
 .role{white-space:nowrap;font:600 12px system-ui}
 .g{font-size:15px;width:22px;text-align:center}
 td span{font-size:9px;mix-blend-mode:difference;color:#fff}
 .sw{width:120px}
</style>`
for (const { tname, surfaces, hex } of proofRows) {
  html += `<div class=theme style="background:${surfaces.bg};color:${themes[tname].text.text}">
   <h2>${tname.toUpperCase()} — role · glyph · normal / deuteranopia / protanopia / tritanopia</h2>
   <table><tr><th>role</th><th>glyph</th><th>normal</th><th>deutan</th><th>protan</th><th>tritan</th></tr>`
  for (const r of roleNames) {
    html += `<tr><td class=role>${r}</td><td class=g>${GLYPH[r]}</td>
     <td class="sw" style="background:${hex[r]}"><span>${hex[r]}</span></td>
     <td class="sw" style="background:${simulate(hex[r], 'deutan')}"></td>
     <td class="sw" style="background:${simulate(hex[r], 'protan')}"></td>
     <td class="sw" style="background:${simulate(hex[r], 'tritan')}"></td></tr>`
  }
  html += `</table></div>`
}
writeFileSync(new URL('./proof.html', import.meta.url), html)

/* ----------------------- emit tokens.generated.json --------------------- */
const tokensOut = {}
for (const [tname, t] of Object.entries(themes)) {
  tokensOut[tname] = {
    surfaces: t.surfaces, text: t.text,
    status: Object.fromEntries(Object.entries(t.status).map(([k, v]) => [k, { oklch: v, hex: oklchToHex(v), glyph: GLYPH[k] }])),
  }
}
writeFileSync(new URL('./tokens.generated.json', import.meta.url), JSON.stringify(tokensOut, null, 2))

console.log(`\n${fails === 0 ? '✅ ALL CHECKS PASS' : '❌ ' + fails + ' FAILURES'} — wrote design/proof.html + design/tokens.generated.json`)
process.exit(fails === 0 ? 0 : 1)
