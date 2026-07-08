# Nexus Design System

> Living design-system doc. **Stage A (color language + token architecture) is verified & locked** (2026-06-05).
> Spacing/type/density/chrome/honest-state detail lives in `tasks/specs/UI-P0-foundation.md`.
> Authoritative token values: `ui/design/tokens.generated.json` · visual + CVD proof: `ui/design/proof.png`
> · verifier (the gate): `ui/design/verify.mjs` (`node ui/design/verify.mjs`).

## Token architecture (3 tiers, OKLCH, dark-first)

1. **Reference** (theme-agnostic OKLCH ramps) — raw primitives, never used directly.
2. **Semantic / domain** (per-theme, the `[data-theme]` blocks) — encode *meaning*, not generic primary/secondary:
   surfaces/text/accent + the status roles below. Themes swap via a single `data-theme` attribute on `<html>`.
3. **Component** (density-derived) — `--roster-row-h`, `--now-bar-h`, etc.

Three themes: **dark** (canonical), **light** (derived), **amber** (night-vision, first-class).
Migration is additive + back-compat aliases (`--state-good` → `--snr-strong`) so existing CSS keeps working.

## The one semantic color language

Applied **identically** across roster dots, map, log/awards badges, decode rows, the Now-Bar verdict.
**Color is a redundant cue; the glyph is the primary, CVD-immune identifier.**

| role | meaning | glyph | dark | light | amber |
|---|---|---|---|---|---|
| `new-entity` | ATNO — never worked (top priority) | ★ | `#ffb35b` | `#a30000` | `#ffcd62` |
| `new-band` | worked entity, new band-slot | ◑ | `#ccb10d` | `#bb8805` | `#d88027` |
| `new-mode` | worked entity, new mode-slot | ◧ | `#bb8aef` | `#7c44c3` | `#f9875e` |
| `worked` | worked, unconfirmed | ○ | `#7a96ab` | `#5c7f98` | `#98724e` |
| `confirmed` | confirmed (LoTW/eQSL/QSL) | ✓ | `#70e093` | `#008d3c` | `#d8bd51` |
| `dupe` | already have it (recede) | · | `#747b81` | `#98a0a5` | `#675849` |
| `snr-strong` | strong signal | ▇ | `#69d98d` | `#007f35` | `#e1c34b` |
| `snr-marginal` | marginal signal | ▅ | `#f1ca47` | `#a27000` | `#dd9231` |
| `snr-weak` | weak signal | ▂ | `#ec5b57` | `#a20003` | `#c13f29` |
| `tx` | transmitting | ▲ | `#e64343` | `#a50000` | `#c13f29` |
| `rx` | receiving | ▼ | `#69d98d` | `#007f35` | `#e1c34b` |
| `band-open` | path workable now | ● | `#69d98d` | `#007f35` | `#e1c34b` |
| `band-marginal` | path marginal | ◐ | `#f1ca47` | `#a27000` | `#dd9231` |
| `band-closed` | path closed (recede) | ⊘ | `#747b81` | `#80878d` | `#675849` |
| `alert-critical` | interrupt — act now | ⚑ | `#ff6100` | `#9c0000` | `#ff964e` |
| `alert-warning` | caution | △ | `#f7c243` | `#a76d00` | `#f0903c` |
| `alert-info` | informational | i | `#79c0f1` | `#0070a6` | `#b88848` |

### Rules (enforced by `verify.mjs`)
1. **Color + glyph, always.** Never color alone (≈8% of male hams have red-green CVD). Every role's glyph is **unique**.
2. **Color means one *family* of meaning:** green = good, red = bad/transmit, amber/gold = caution/opportunity,
   blue = info. Roles that share a meaning-family **share a hue** and are disambiguated by glyph + the column/surface
   they live in.
3. **Good (green) ↔ bad (red) must survive every CVD** — guaranteed by keeping greens lighter than reds, so deutan/
   protan (which collapse hue) still separate them by luminance. (Verified ΔE ≥ 0.06 under Machado deutan/protan/tritan.)
4. **Amber (monochromatic gamut)** cannot make distinct green/red *hues*, so good = **bright**, bad = **dark**, always
   with a glyph.
5. **`alert-critical` is an interrupt, not a "bad signal"** — it escapes the good/bad lightness convention: it is the
   **brightest/loudest** treatment (distinct from `tx`), and is always rendered as a **filled, bordered chip + ⚑**
   (+ pulse when motion is allowed). In amber it is forced bright (not buried).
6. **ATNO is the loudest need-state** (highest contrast of the need-set) in every theme.
7. **De-emphasis states** (`dupe`, `band-closed`) intentionally sit below the 3:1 status floor — they *recede*, are
   glyph/position-carried, and are **never the sole carrier of must-read text** (a dupe call sign uses the normal text
   token at reduced opacity, not the dim status color).

### Layout constraints these rules imply (must hold in components)
- `confirmed` / `snr-strong` / `rx` / `band-open` are the **same green** — only safe because each lives in its own
  column/context (log badge vs SNR column vs TX-RX indicator vs Now-Bar band state). Never render two of them adjacent
  without their label/column.
- `tx` and `snr-weak` share the **red** band — disambiguated by column (TX-RX chrome vs SNR column) + glyph.
- `tx` vs `alert-critical`: both reddish, but distinguished by **form** (a bare ▲ state indicator vs a filled ⚑ chip)
  + the brighter/oranger critical color — robust under all CVD.

## Verification (the gate — all PASS)
`node ui/design/verify.mjs` computes, from the OKLCH values: WCAG 2.1 contrast (text ≥ 4.5:1, status ≥ 3:1),
Machado-2009 CVD simulation (deuteranopia/protanopia/tritanopia, severity 1.0, in linear RGB), OKLab ΔE distances,
and the salience ordering. Gates: text/status contrast · good↔bad CVD ΔE ≥ 0.06 · need-set unique glyphs + colour
ΔE ≥ 0.03 · **glyph uniqueness across all 17 roles** · **ATNO loudest need** · **alert-critical never buried**.
De-emphasis states are exempt from the 3:1 floor by design. The proof sheet (`proof.png`) renders every role under
normal + all three CVD types, per theme, for visual sign-off.

### Adversarial-review record (what a skeptical second pass caught, and the fix)
A focused adversarial review found gaps the first gate was structurally blind to — all now fixed and gated:
- **`tx` == `alert-critical`** (byte-identical in light/amber) → split: critical is a distinct bright orange + filled
  ⚑ chip; documented form-based distinction; gate added.
- **Amber `alert-critical` was nearly the dimmest role** → forced bright (L 0.82); "never buried" gate added.
- **Glyph reuse** (`○` worked *and* band-closed; `▲` tx *and* alert-warning) → band-closed→`⊘`, alert-warning→`△`;
  all-roles glyph-uniqueness gate added.
- **ATNO not the loudest need-state** → re-laddered; "ATNO loudest" gate added.
- Honest framing recorded: **red carries two meanings** (transmit + bad/alert), separated by glyph + context — not
  claimed as one-meaning purity.

## Perceptual colormaps (for the waterfall / map / heatmaps)
Named LUTs (data + sampler in `ui/src/colormaps.ts`, Stage-A deliverable): **inferno** (default), **viridis**,
**cividis** (CVD-safe), **turbo**, **classic-SDR-green**, **amber-CRT**. Luminance-monotonic by construction (fixes
the current non-monotonic `t*t`/`t*t*t` waterfall palette). Consumed by P1 (waterfall) / P2 (map) / the likelihood
heatmap.

## Open Stage-A implementation items (post sign-off)
Spacing ladder, type scale, density system (`data-density` guided/standard/dense), reduced-motion, the Radix chrome kit
+ Lucide, honest-state provenance, and the WCAG-2.2 pass — all specified in `tasks/specs/UI-P0-foundation.md`. This doc
covers the **color language + token architecture** that gates everything else.
