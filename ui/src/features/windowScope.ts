// Window/surface identity — the one place that decides which storage keys are shared
// between windows and which are private to this one.
//
// A SURFACE is (view, instance) — see `surfaceId`, which is what per-surface storage keys
// off; a WINDOW hosts exactly one surface. `instance` is an
// opaque VALIDATED TOKEN: `main` (the docked surface in the main window), `w<n>` (an
// extra unbound surface — a torn-off panel or a second-monitor board) or `r<id>` (bound
// to a RadioProfile.id; multi-radio, not reachable yet). Nothing outside this file ever
// interprets it — it is a key component only.
//
// Panel VISIBILITY is SURFACE-scoped, deliberately. An app-global flag is exactly what
// makes a docked and a popped-out copy of the same view fight over one value — the
// `nexus.waterfall.detached` defect this replaces.
//
// ## `main` writes the BARE key. This is the whole zero-migration property.
//
// Every key routed through here except `nexus.panels.*` already exists on operators'
// disks, unsuffixed, written by builds that had never heard of an instance. So the
// primary surface must keep producing the byte-identical string — `tempo-right-rail-w`,
// not `tempo-right-rail-w.main` — or an upgrade silently orphans every saved rail width,
// zoom pin, map projection and board filter. That mirrors `chains.rs::panel_label`, which
// makes the same promise for window LABELS (`panel-{slug}` for Main, suffixed above it),
// and `windowScope.test.ts` pins it as literal strings rather than as a property.
//
// `nexus.panels.*` is the one exception and it goes the other way: it shipped in 0.15.0
// ALREADY suffixed (`nexus.panels.operate.main`), so for that key the byte-identical
// string is the suffixed one. It therefore builds its key itself — see panelState.ts.
//
// ## A new surface INHERITS, then diverges
//
// `surfaceGet` falls back to the bare (primary) key when this surface has never written
// one. Without that, popping a panel out would reset it to first-run defaults — which is
// a regression, not a feature: `nexus.connect.projection` exists PRECISELY so a torn-off
// map opens on the globe the operator was already using (see MapView.tsx). Inheriting
// costs nothing in the other direction, because the first write makes the surface its own.

/** `main` | `w<n>` | `r<id>`. Rejected on mismatch, never silently repaired.
 *
 *  Digits must be CANONICAL (no leading zeros), matching `chains.rs::canonical_digits`
 *  exactly — bounds included (`w` ≤ 3 digits, `r` ≤ 9). Without that, `r02` and `r2` are
 *  two tokens, hence two sets of storage keys, for ONE rig. Rust refuses to open such a
 *  window at all, so this is belt-and-braces; a grammar that is looser on one side of the
 *  wire than the other is how the two drift. */
const INSTANCE_RE = /^(main|w(0|[1-9][0-9]{0,2})|r(0|[1-9][0-9]{0,8}))$/

export function isInstanceToken(v: unknown): v is string {
  return typeof v === 'string' && INSTANCE_RE.test(v)
}

/**
 * This window's instance token, read from the URL. An explicit `?instance=` wins; anything
 * else is `main`. `instance` is its OWN parameter — never baked into the panel slug, which
 * the Rust side alnum-filters (so `operate-2` / `operate:2` / `operate2` would all collapse).
 *
 * NOTE this is the instance HALF of the address only. It is NOT a surface id and must not be
 * used to scope storage — see [`surfaceId`], and the bug note there for why.
 */
export function windowInstance(): string {
  const raw = search().get('instance')
  return isInstanceToken(raw) ? raw : 'main'
}

/**
 * The query string, or an empty one when there is no DOM.
 *
 * `features/paneLayout.ts` is deliberately pure — its tests run without jsdom, the same way
 * `features/state.ts` does — so reading `window.location` unguarded turns a storage helper
 * into a hard dependency on a browser. Outside a window there is no surface, and the primary
 * surface (`main`, the bare key) is the correct answer: it is what every caller wants when
 * asking "which window am I?" has no meaning. Never silently wrong in a browser, where
 * `window.location` always exists.
 */
function search(): URLSearchParams {
  try {
    return new URLSearchParams(window.location.search)
  } catch {
    return new URLSearchParams()
  }
}

/**
 * This window's SURFACE id — the thing per-surface storage is actually keyed by.
 *
 * A surface is `(view, instance)`, exactly as `chains.rs::panel_key` splits a window label
 * into `(slug, instance)`. **Both halves matter**, and the view half is the one that
 * discriminates today: `open_panel_window` sends no `&instance=` for a `main` surface, and
 * every pop-out that can be opened right now IS `main` — so the instance alone is constant
 * across all of them.
 *
 * Collapsing every pop-out to one token was a real, live collision, not a theoretical one:
 * a torn-off `waterfall` and a torn-off `operate` (which embeds a waterfall) both wrote the
 * same zoom key, and `bandmapCw` + `bandmapPhone` both wrote the same spot-legend key. Two
 * pop-outs fighting each other is the same defect as a pop-out fighting the main window, and
 * the goal here was to remove it, not to relocate it.
 *
 *   main window                        -> `main`   (the BARE key — zero migration)
 *   ?panel=waterfall                   -> `waterfall`
 *   ?panel=waterfall&instance=w2       -> `waterfall.w2`
 *
 * One window per panel slug is enforced in Rust (`open_panel_window` focuses an existing
 * window rather than opening a second), so the slug is a sufficient discriminator today and
 * the instance suffix only starts doing work when `w<n>` surfaces are un-gated.
 */
export function surfaceId(): string {
  const panel = search().get('panel')
  if (!panel) return 'main'
  const inst = windowInstance()
  return inst === 'main' ? panel : `${panel}.${inst}`
}

export type KeyScope = 'global' | 'surface' | 'radio'

/**
 * Scope a storage key.
 * - `global`  — shared by every window (an app-wide preference).
 * - `surface` — private to this window's (view, instance).
 * - `radio`   — shared by every surface driving the same rig. Only an `r<id>` window is
 *   radio-bound, so every other surface shares the primary rig's key.
 *
 * The PRIMARY surface of either scope (`main`, and any non-`r` surface for `radio`) gets
 * the base key UNCHANGED — see the zero-migration note at the top. That also makes the
 * eventual `surface` → `radio` promotion of the tune-step / waterfall-calibration keys
 * free: they are bare today and stay bare until an `r<id>` window actually exists.
 */
export function scopedKey(base: string, scope: KeyScope, instance?: string): string {
  if (scope === 'global') return base
  if (scope === 'surface') {
    // Keyed by the SURFACE (view + instance), not the instance alone — see `surfaceId`.
    const id = instance ?? surfaceId()
    return id === 'main' ? base : `${base}.${id}`
  }
  // `radio` scope keys off the INSTANCE only: two views driving the same rig must agree, so
  // the view half is deliberately dropped here.
  const inst = instance ?? windowInstance()
  return inst.startsWith('r') ? `${base}.${inst}` : base
}

/** [`scopedKey`] for the per-surface scope — the form nearly every call site wants. */
export function surfaceKey(base: string, instance?: string): string {
  return scopedKey(base, 'surface', instance)
}

/**
 * Read a per-surface value, falling back to the primary surface's value while this
 * surface has never written one (see the inheritance note above). Storage-safe: a
 * blocked/unavailable store reads as "nothing stored", never as a throw.
 */
export function surfaceGet(base: string): string | null {
  const key = surfaceKey(base)
  try {
    const own = window.localStorage.getItem(key)
    if (own != null || key === base) return own
    return window.localStorage.getItem(base)
  } catch {
    return null
  }
}

/** Write a per-surface value. Never touches another surface's key. */
export function surfaceSet(base: string, value: string): void {
  try {
    window.localStorage.setItem(surfaceKey(base), value)
  } catch {
    /* full/unavailable — the value still applies for this session */
  }
}
