#!/usr/bin/env node
// Generate the callsign → US-state index for the WAS "New State" hint.
//
// A DX-cluster / CW / SSB spot carries NO grid — only a callsign. This index answers
// callsign → state directly (from the FCC licensee address), so New-State lights up across the
// whole spot firehose, precisely (no coarse 4-char grid cell to guess). It is a HINT (the
// licensee's mailing state); the live decode grid refines it for rovers, and actual WAS credit
// still comes from the confirmed QSO's logged ADIF STATE.
//
// Source: the FCC ULS COMPLETE amateur file l_amat.zip (~194 MB, refreshed weekly on Sundays):
//   https://data.fcc.gov/download/pub/uls/complete/l_amat.zip
//   - HD.dat: license header — field[1]=USI, field[5]=license_status ('A' = active).
//   - EN.dat: entity/address — field[1]=USI, field[4]=call_sign, field[5]=entity_type ('L' =
//     licensee), field[17]=state.
// Join EN(licensee) to the active USIs from HD, keep valid WAS states, dedup by callsign.
//
// Output (matches crates/propagation/src/fccstate.rs FccStates::load):
//   fcc-states.bin  — 16-byte header (magic "NEXFCCS1" + count:u32 LE + 4 reserved) then
//                     `count` callsign-sorted 7-byte entries (6-char space-padded UPPER call +
//                     1 state byte = WAS_STATES index+1). ~5 MB for ~750k hams.
//   fcc-states.json — manifest {format, generated, source, count} the client checks to refresh.
//
// Run:  node scripts/gen-fcc-states.mjs <l_amat.zip | --download> [outDir]
//   --download fetches the current weekly file to a temp path first.
// Needs `curl` + `python3` on PATH (both present on CI runners). This is what the weekly Action runs.

import { createReadStream, writeFileSync, mkdtempSync, existsSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { createInterface } from 'node:readline'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

// MUST match crates/propagation/src/awards.rs WAS_STATES order exactly (index+1 = state byte).
const WAS_STATES = [
  'AK', 'AL', 'AR', 'AZ', 'CA', 'CO', 'CT', 'DE', 'FL', 'GA', 'HI', 'IA', 'ID', 'IL', 'IN', 'KS',
  'KY', 'LA', 'MA', 'MD', 'ME', 'MI', 'MN', 'MO', 'MS', 'MT', 'NC', 'ND', 'NE', 'NH', 'NJ', 'NM',
  'NV', 'NY', 'OH', 'OK', 'OR', 'PA', 'RI', 'SC', 'SD', 'TN', 'TX', 'UT', 'VA', 'VT', 'WA', 'WI',
  'WV', 'WY',
]
const CODE = new Map(WAS_STATES.map((s, i) => [s, i + 1]))
const FCC_URL = 'https://data.fcc.gov/download/pub/uls/complete/l_amat.zip'
const CALL_LEN = 6

async function eachLine(path, fn) {
  const rl = createInterface({ input: createReadStream(path), crlfDelay: Infinity })
  for await (const line of rl) fn(line)
}

async function main() {
  const args = process.argv.slice(2)
  let zip = args[0]
  const outDir = args[1] || process.cwd()
  if (!zip) {
    console.error('usage: gen-fcc-states.mjs <l_amat.zip | --download> [outDir]')
    process.exit(2)
  }
  const work = mkdtempSync(join(tmpdir(), 'fccstate-'))
  if (zip === '--download') {
    zip = join(work, 'l_amat.zip')
    console.error(`downloading ${FCC_URL} …`)
    execFileSync('curl', ['-fsSL', '-o', zip, FCC_URL], { stdio: ['ignore', 'ignore', 'inherit'] })
  }
  if (!existsSync(zip)) throw new Error(`no such file: ${zip}`)

  console.error('extracting EN.dat + HD.dat …')
  // Extract via python3's built-in zipfile (no `unzip` dependency — portable to CI + dev boxes).
  execFileSync(
    'python3',
    ['-c', 'import zipfile,sys\nz=zipfile.ZipFile(sys.argv[1])\nfor n in ("EN.dat","HD.dat"): z.extract(n, sys.argv[2])', zip, work],
    { stdio: 'ignore' },
  )

  // Pass 1: active license USIs (HD field[5] === 'A').
  const active = new Set()
  await eachLine(join(work, 'HD.dat'), (l) => {
    const f = l.split('|')
    if (f[0] === 'HD' && f[5] === 'A' && f[1]) active.add(f[1])
  })
  console.error(`  active licenses: ${active.size.toLocaleString()}`)

  // Pass 2: licensee (entity_type 'L') call → state, gated on an active USI + a valid WAS state.
  const byCall = new Map()
  await eachLine(join(work, 'EN.dat'), (l) => {
    const f = l.split('|')
    if (f[0] !== 'EN' || f[5] !== 'L') return
    const usi = f[1]
    const call = (f[4] || '').trim().toUpperCase()
    const st = (f[17] || '').trim().toUpperCase()
    if (!call || call.length > CALL_LEN || !active.has(usi)) return
    const code = CODE.get(st)
    if (code) byCall.set(call, code) // last licensee record wins
  })
  console.error(`  US amateur callsign→state entries: ${byCall.size.toLocaleString()}`)

  // Serialize: header + callsign-sorted 7-byte entries.
  const calls = [...byCall.keys()].sort()
  const buf = Buffer.alloc(16 + calls.length * (CALL_LEN + 1))
  buf.write('NEXFCCS1', 0, 'ascii')
  buf.writeUInt32LE(calls.length, 8)
  let off = 16
  for (const call of calls) {
    buf.write(call.padEnd(CALL_LEN, ' '), off, 'ascii')
    buf.writeUInt8(byCall.get(call), off + CALL_LEN)
    off += CALL_LEN + 1
  }
  const binPath = join(outDir, 'fcc-states.bin')
  writeFileSync(binPath, buf)
  const manifest = {
    format: 'NEXFCCS1',
    generated: new Date().toISOString(),
    source: 'FCC ULS complete amateur file (l_amat.zip)',
    count: calls.length,
    bytes: buf.length,
  }
  writeFileSync(join(outDir, 'fcc-states.json'), JSON.stringify(manifest, null, 2) + '\n')
  console.error(`wrote ${binPath} (${(buf.length / 1e6).toFixed(1)} MB, ${calls.length.toLocaleString()} calls)`)
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
