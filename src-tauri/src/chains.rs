//! Window → radio-chain ADDRESSING (multi-radio phase 1b).
//!
//! This module answers exactly one question — *which radio does this window talk to?* — and
//! deliberately answers nothing else. It is **inert at runtime**: [`Chains`] holds exactly one
//! chain, no command consults it yet, and no caller passes an instance token yet.
//!
//! ## The address
//!
//! A surface is `(panel, instance)`. The instance is a validated TOKEN, never a bare number:
//!
//! | token   | meaning                                                             |
//! |---------|---------------------------------------------------------------------|
//! | `main`  | the docked surface in the main window                               |
//! | `w<n>`  | an extra UNBOUND surface (second-monitor board, torn-off sub-panel)  |
//! | `r<id>` | BOUND to the radio whose `RadioProfile::id` is `<id>`                |
//!
//! Grammar: `^(main|w[0-9]{1,3}|r[0-9]{1,9})$`, and additionally the digits must be canonical
//! (no leading zeros) so that token → label is a bijection. Without that, `r02` and `r2` are two
//! labels, two saved geometry files and two windows for **one** rig.
//!
//! **Why a token and not a `u32`.** A `u32` cannot tell `w2` from `r2`. Given the bare label
//! `panel-operate-2`, a resolver looking for a radio finds none and falls back to the primary
//! chain — rendering and *commanding* radio 1 in a window the operator opened for radio 2. That
//! is a wrong-rig bug, not a display bug, and the token makes it unrepresentable.
//!
//! **Why the chain comes from the WINDOW LABEL and not a command argument.** Same reason, one
//! level up: a call site that forgets to pass an argument would silently target the wrong
//! transmitter. The label is attached by the OS to the window the click happened in, so it
//! cannot be omitted. See [`chain_of`].
//!
//! **Ids are `RadioProfile::id`s, not a parallel numbering.** `r<id>` names a profile id
//! (`crates/tempo-app/src/settings.rs` — *"Stable id, never reused"*), so [`Chains`] is keyed by
//! the same ids. A migrated single-radio station is profile **0**, so chain ids start at 0 and
//! there is no synthetic "chain 1" constant anywhere.
//!
//! ## Why this module denies warnings on its own
//!
//! `src-tauri` is its own workspace root, so it was never linted by CI and carries a
//! pre-existing backlog (15 warnings at the time of writing: MSRV, type complexity, redundant
//! casts). `-D warnings` on the whole crate would therefore fail on day one, and burning that
//! backlog down is unrelated churn in a 10k-line `lib.rs`.
//!
//! So the deny is scoped HERE instead. Every wrong-rig property this step claims lives in this
//! file, and a lint regression in it is exactly what nobody would notice. The crate-wide gate is
//! backlog, tracked in `docs/multi-radio-rollback.md`.
#![deny(warnings, clippy::all)]

use crate::SharedEngine;
use std::collections::BTreeMap;

/// How many receive/transmit chains may be registered. **One, deliberately** — see
/// [`Chains::add`] for why this is the whole point of the step rather than a limitation.
pub(crate) const MAX_CHAINS: usize = 1;

/// The instance half of a `(panel, instance)` surface address. Parsed from, and rendered back
/// to, the token grammar in the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Instance {
    /// The docked surface in the main window. Renders as no label suffix at all, which is what
    /// makes every window label and geometry file already on disk keep resolving.
    Main,
    /// An extra unbound surface. Follows the station's active radio; names no chain.
    Window(u32),
    /// Bound to `RadioProfile::id`.
    Radio(u32),
}

impl std::fmt::Display for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Instance::Main => f.write_str("main"),
            Instance::Window(n) => write!(f, "w{n}"),
            Instance::Radio(id) => write!(f, "r{id}"),
        }
    }
}

/// Parse the digits of a `w`/`r` token. `max_digits` bounds the grammar; leading zeros are
/// rejected so `Display` is an exact inverse of `parse` (see the module docs).
fn canonical_digits(digits: &str, max_digits: usize) -> Option<u32> {
    if digits.is_empty() || digits.len() > max_digits {
        return None;
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    digits.parse().ok()
}

impl Instance {
    /// Parse an instance token, **rejecting** anything malformed rather than coercing it.
    ///
    /// Rejection is the point. `open_panel_window` already silently strips non-alphanumerics
    /// from the panel slug, which is exactly why `operate-2`, `operate:2` and `operate2` all
    /// collapse onto one window today. Repeating that here would let a typo'd token quietly
    /// address a different radio.
    pub(crate) fn parse(token: &str) -> Result<Self, String> {
        let bad = || {
            format!(
                "invalid window instance {token:?} — expected `main`, `w<n>` (an extra unbound \
                 surface, n < 1000) or `r<id>` (bound to a radio profile id). Digits must be \
                 canonical: `r02` is rejected so it cannot become a second window for radio 2."
            )
        };
        if token == "main" {
            return Ok(Instance::Main);
        }
        if let Some(digits) = token.strip_prefix('w') {
            return canonical_digits(digits, 3).map(Instance::Window).ok_or_else(bad);
        }
        if let Some(digits) = token.strip_prefix('r') {
            return canonical_digits(digits, 9).map(Instance::Radio).ok_or_else(bad);
        }
        Err(bad())
    }

    /// The chain this instance names, or `None` when the surface is **unbound**.
    ///
    /// `main` and `w<n>` are unbound by construction: they follow whatever radio the station has
    /// active. Only `r<id>` names a chain.
    pub(crate) fn chain(self) -> ChainRef {
        match self {
            Instance::Radio(id) => ChainRef::Radio(id),
            Instance::Main | Instance::Window(_) => ChainRef::ActiveRadio,
        }
    }

    /// Whether a window with this instance may PERSIST its size/position across restarts.
    ///
    /// `w<n>` may not. Those ids are allocated dynamically as sub-panels are torn off, so they
    /// are recycled — a later `w3` would inherit a stranger's saved rect and open somewhere the
    /// operator never put it. `main` and `r<id>` are stable, so they persist.
    pub(crate) fn persists_geometry(self) -> bool {
        !matches!(self, Instance::Window(_))
    }
}

/// Which chain a surface addresses.
///
/// **This is an enum and not an `Option<u32>` on purpose.** `Option` reads as "a chain id, or
/// nothing", and the obvious way to consume that is `.unwrap_or(0)` — but `0` is not a neutral
/// sentinel here. It is the REAL `RadioProfile::id` of the default profile that
/// `ensure_radio_profiles` seeds for every migrated single-radio station. So `.unwrap_or(0)`
/// passes every test on a default station and silently commands the wrong rig on any station
/// whose `active_radio != 0` — the same wrong-rig bug the token grammar exists to prevent,
/// walking back in through the return type.
///
/// [`ChainRef::ActiveRadio`] cannot be mistaken for an id, so the fallback has to be written out
/// deliberately at the one call site that knows which radio is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChainRef {
    /// Unbound: follow whatever radio the station currently has active. The main window, a
    /// `w<n>` surface, and any label this app did not create.
    ActiveRadio,
    /// Bound to this `RadioProfile::id`.
    Radio(u32),
}

/// The OS window label for a surface.
///
/// `panel-{slug}` for `main` — byte-identical to every label and every
/// `bandmap-window-<slug>.json` already on disk, so there is zero migration — and
/// `panel-{slug}-{instance}` above it.
pub(crate) fn panel_label(slug: &str, inst: Instance) -> String {
    match inst {
        Instance::Main => format!("panel-{slug}"),
        other => format!("panel-{slug}-{other}"),
    }
}

/// Split a window label back into `(slug, instance)`. The inverse of [`panel_label`].
///
/// `None` for any label this app did not create as a panel window (the main window, a foreign
/// or hand-edited label).
///
/// The split rule is **"the tail matches `^(w|r)[0-9]+$`"**, NOT "the tail parses as a number".
/// That distinction is the whole amendment: a numeric rule cannot tell `w2` from `r2`. It is
/// unambiguous because the slug is alphanumeric-filtered where the window is created and so can
/// never itself contain a `-` — which is also why a panel whose slug legitimately ends in a
/// digit (`panel-r2`) still reads as the slug `r2` on `main`, not as instance `r2`.
pub(crate) fn panel_key(label: &str) -> Option<(&str, Instance)> {
    let rest = label.strip_prefix("panel-")?;
    let (slug, inst) = match rest.rsplit_once('-') {
        Some((head, tail)) if looks_like_instance(tail) => (head, Instance::parse(tail).ok()?),
        _ => (rest, Instance::Main),
    };
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((slug, inst))
}

/// `^(w|r)[0-9]+$` — the shape that makes a label tail an instance rather than part of the slug.
/// Deliberately looser than [`Instance::parse`]: shape decides the SPLIT, validity decides
/// acceptance, so `panel-operate-w9999` is rejected outright instead of being re-read as the
/// nonsense slug `operate-w9999`.
fn looks_like_instance(tail: &str) -> bool {
    tail.strip_prefix(['w', 'r'])
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// The chain a window addresses, derived from its LABEL — never from a command argument, so a
/// call site cannot forget it and end up on the wrong transmitter.
///
/// [`ChainRef::ActiveRadio`] means **unbound**: the main window, a `main` surface, a `w<n>`
/// surface, or a label this app did not create. Every one of those follows the station's active
/// radio, so merging them is safe — the dangerous direction is the other one, naming a radio
/// that was not asked for.
// Inert until commands become chain-scoped (explicitly not this step). Kept here, next to the
// grammar it depends on, so the resolver and the token can never drift apart.
#[allow(dead_code)]
pub(crate) fn chain_of(window: &tauri::WebviewWindow) -> ChainRef {
    chain_of_label(window.label())
}

/// [`chain_of`] over a bare label — the whole logic, testable without a live window.
pub(crate) fn chain_of_label(label: &str) -> ChainRef {
    panel_key(label).map_or(ChainRef::ActiveRadio, |(_slug, inst)| inst.chain())
}

/// May a window for this instance actually be opened yet?
///
/// Both non-`main` instances are refused, for the SAME reason rather than one being a lesser
/// case: the plumbing a second surface needs does not exist, and each would fail in a way the
/// operator would read as a Nexus bug rather than a missing feature.
///
/// * `r<id>` — `subscribeSnapshot` returns one station-wide `AppSnapshot` with one `radio`
///   block, and no command takes a radio id. So the window would render the ACTIVE radio and
///   its Stop TX would halt the ACTIVE radio. That is a wrong-rig abort.
/// * `w<n>` — storage cross-talk is now FIXED (`ui/src/features/windowScope.ts` keys per-surface
///   state by `(view, instance)`), so this is no longer the blocker it was. It stays refused for
///   a narrower, still-real reason: `w<n>` ids are recycled — `Instance::persists_geometry`
///   already refuses to save window geometry for exactly that reason — so a later `w3` of the
///   same view would inherit a closed `w3`'s split fractions, zoom pin and filters. The storage
///   side has no equivalent sweep yet. That sweep belongs beside `redockStalePopouts` in
///   `ui/src/main.tsx`, in the same change that lifts this arm.
///
/// This is a separate function purely so it is testable — the caller is a `#[tauri::command]`
/// needing a live `AppHandle`, and a gate whose whole job is refusing is worth a test that
/// proves it refuses.
pub(crate) fn openable(inst: Instance) -> Result<(), String> {
    match inst {
        Instance::Main => Ok(()),
        Instance::Radio(id) => Err(format!(
            "radio-bound panel windows (r{id}) are not available yet: the snapshot and every \
             command are still station-wide, so the window would silently show and COMMAND the \
             active radio instead of radio {id}. Refused deliberately rather than shipped as a \
             wrong-rig hazard."
        )),
        Instance::Window(n) => Err(format!(
            "extra panel surfaces (w{n}) are not available yet: surface ids are recycled and \
             nothing sweeps a closed surface's saved layout, so this window would open with a \
             previous one's splits, zoom and filters."
        )),
    }
}

/// The station's receive/transmit chains, keyed by `RadioProfile::id`.
///
/// **Hard-capped at [`MAX_CHAINS`].** See [`Chains::add`].
// Deliberately NOT `.manage()`d — see the `Chains::new` call site's absence in lib.rs and
// `docs/multi-radio-rollback.md`. The registry would have to be keyed by `RadioProfile::id`,
// and the only id available at boot is a snapshot of `settings.active_radio` that goes stale
// the moment the operator switches radios. So it is built and PROVEN here, and acquires its
// first reader on the cap-lift branch, where re-keying on radio-switch is problem #1.
#[allow(dead_code)]
pub(crate) struct Chains {
    chains: BTreeMap<u32, SharedEngine>,
}

impl Chains {
    /// The registry as it exists in production: exactly one chain, the operator's active radio.
    // No production caller while the registry is unmanaged — the cap-lift adds one.
    #[allow(dead_code)]
    pub(crate) fn new(primary_id: u32, primary: SharedEngine) -> Self {
        let me = Self {
            chains: BTreeMap::from([(primary_id, primary)]),
        };
        // The cap is enforced in `add`, but `new` is the OTHER way a registry comes into
        // existence and the obvious next edit — taking the whole profile list — bypasses `add`
        // entirely. No existing test would catch that, because they all go through `add`. The
        // standard here is that a cap which is documented but not enforced is worse than none.
        debug_assert!(me.chains.len() <= MAX_CHAINS, "Chains::new exceeded the cap");
        me
    }

    /// Register another chain. **This always fails today, and that is the point.**
    ///
    /// Everything in the multi-radio foundation up to and including this registry is inert at
    /// runtime, which is what makes the whole program safe to abandon by simply stopping. Lifting
    /// this cap is the single documented line where that stops being true — two decoders, two
    /// rigs, contended locks and TX arbitration all arrive at once — so the lift goes on the
    /// `multiradio-live` branch, ships with a runtime kill switch, and is not part of addressing.
    /// See `docs/multi-radio-rollback.md`.
    ///
    /// It errors loudly rather than no-op'ing or truncating so a future caller cannot mistake a
    /// deliberate refusal for a silent success.
    // No production caller: nothing may add a chain while the cap stands.
    #[allow(dead_code)]
    pub(crate) fn add(&mut self, id: u32, engine: SharedEngine) -> Result<(), String> {
        if self.chains.contains_key(&id) {
            return Err(format!(
                "chain {id} is already registered; replacing it would swap the engine out from \
                 under every command already holding the old one"
            ));
        }
        if self.chains.len() >= MAX_CHAINS {
            return Err(format!(
                "refusing to register chain {id}: the chain registry is HARD-CAPPED at \
                 {MAX_CHAINS}. This is deliberate, not an unfinished feature — the multi-radio \
                 foundation is inert at runtime precisely because a second chain cannot spawn, \
                 and lifting the cap is the documented line where that stops being true (two \
                 decoders, two rigs, contended locks, TX arbitration). The lift belongs on the \
                 `multiradio-live` branch, behind a runtime kill switch, with both golden \
                 fixtures still green. See docs/multi-radio-rollback.md."
            ));
        }
        self.chains.insert(id, engine);
        Ok(())
    }

    /// The engine driving chain `id`, or `None` when no such chain exists.
    // No production caller yet: wiring the ~97 Tauri commands to a chain is a later step.
    #[allow(dead_code)]
    pub(crate) fn get(&self, id: u32) -> Option<&SharedEngine> {
        self.chains.get(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_app::engine::Engine;
    use tempo_app::settings::Settings;

    fn engine() -> SharedEngine {
        std::sync::Arc::new(std::sync::Mutex::new(Engine::with_settings(
            Settings::default(),
        )))
    }

    // THE bug this step exists to prevent: `w2` and `r2` are different surfaces. A `u32`
    // instance could not tell them apart, so `panel-operate-2` fell back to the primary chain —
    // radio 1 rendered and commanded in a window opened for radio 2.
    #[test]
    fn a_window_instance_is_never_confused_with_a_radio() {
        let w2 = Instance::parse("w2").unwrap();
        let r2 = Instance::parse("r2").unwrap();
        assert_ne!(w2, r2);
        assert_eq!(w2.chain(), ChainRef::ActiveRadio, "w2 is unbound — it names no radio");
        assert_eq!(r2.chain(), ChainRef::Radio(2), "r2 names radio 2");
        assert_ne!(panel_label("operate", w2), panel_label("operate", r2));
        assert_eq!(chain_of_label("panel-operate-w2"), ChainRef::ActiveRadio);
        assert_eq!(chain_of_label("panel-operate-r2"), ChainRef::Radio(2));
    }

    #[test]
    fn instance_tokens_parse_only_the_documented_grammar() {
        assert_eq!(Instance::parse("main").unwrap(), Instance::Main);
        assert_eq!(Instance::parse("w1").unwrap(), Instance::Window(1));
        assert_eq!(Instance::parse("w999").unwrap(), Instance::Window(999));
        // RadioProfile ids start at 0 for a migrated single-radio station.
        assert_eq!(Instance::parse("r0").unwrap(), Instance::Radio(0));
        assert_eq!(
            Instance::parse("r999999999").unwrap(),
            Instance::Radio(999_999_999)
        );
        for bad in [
            "",           // empty
            "2",          // a bare number — the shape the amendment removed
            "W2",         // case matters; the token is generated, not typed
            "w",          // no digits
            "r",          //
            "w1000",      // over the 3-digit window bound
            "r1234567890",// over the 9-digit radio bound
            "r02",        // non-canonical — would be a second label for radio 2
            "w007",       //
            "r2 ",        // stray whitespace
            "r-2",        //
            "radio2",     //
            "main2",      //
            "operate2",   // a panel slug, not an instance
        ] {
            assert!(
                Instance::parse(bad).is_err(),
                "{bad:?} must be REJECTED, not coerced"
            );
        }
    }

    #[test]
    fn rejection_says_what_was_wrong() {
        let err = Instance::parse("r02").unwrap_err();
        assert!(err.contains("r02"), "the bad token is quoted back: {err}");
        assert!(err.contains("canonical"), "the reason is stated: {err}");
    }

    // ZERO MIGRATION: every label and every `bandmap-window-<slug>.json` already on an
    // operator's disk was written before instances existed. They must all still resolve.
    #[test]
    fn legacy_panel_labels_still_resolve_to_main() {
        for slug in [
            "bandmapCw",
            "bandmapPhone",
            "waterfall",
            "operate",
            "needed",
            "connect",
            "dxped",
            "fieldday",
        ] {
            let label = format!("panel-{slug}");
            assert_eq!(panel_key(&label), Some((slug, Instance::Main)), "{label}");
            // …and the label a `main` surface generates is byte-identical to the legacy one.
            assert_eq!(panel_label(slug, Instance::Main), label);
        }
        // A slug that legitimately ENDS IN A DIGIT is not an instance: the separating `-` is.
        assert_eq!(panel_key("panel-r2"), Some(("r2", Instance::Main)));
        assert_eq!(panel_key("panel-w2"), Some(("w2", Instance::Main)));
    }

    #[test]
    fn panel_key_round_trips_every_instance() {
        for inst in [
            Instance::Main,
            Instance::Window(1),
            Instance::Window(999),
            Instance::Radio(0),
            Instance::Radio(2),
            Instance::Radio(999_999_999),
        ] {
            let label = panel_label("bandmapCw", inst);
            assert_eq!(
                panel_key(&label),
                Some(("bandmapCw", inst)),
                "label {label} must decode back to what encoded it"
            );
            // Display is an exact inverse of parse, so the token survives the round trip too.
            assert_eq!(Instance::parse(&inst.to_string()).unwrap(), inst);
        }
    }

    #[test]
    fn panel_key_rejects_labels_this_app_did_not_create() {
        for label in [
            "main",                 // the main window is not a panel window
            "panel-",               // empty slug
            "",                     //
            "operate",              // no `panel-` prefix
            "panel-operate-2",      // the bare-number shape: no longer readable at all
            "panel-operate-main",   // `main` is never suffixed onto a label
            "panel-operate-w9999",  // instance-shaped but out of grammar
            "panel-operate-r02",    // instance-shaped but non-canonical
            "panel-band map",       // non-alphanumeric slug
        ] {
            assert_eq!(panel_key(label), None, "{label:?} must not resolve");
            assert_eq!(
                chain_of_label(label),
                ChainRef::ActiveRadio,
                "{label:?} must name no chain"
            );
        }
    }

    /// The step claims to be inert. `open_panel_window` is the ONE path in it that can conjure a
    /// new OS window, so "inert" has to mean this gate refuses everything above `main` — not
    /// just the radio-bound case that is obviously dangerous.
    #[test]
    fn only_the_main_surface_may_be_opened_yet() {
        assert!(openable(Instance::Main).is_ok(), "the surface that has always existed");
        for inst in [Instance::Radio(0), Instance::Radio(7), Instance::Window(2)] {
            let e = openable(inst).expect_err(&format!("{inst} must be refused while inert"));
            assert!(
                e.contains("not available yet"),
                "refusal must say it is a MISSING FEATURE, not read as a malfunction: {e}"
            );
        }
        // r0 specifically: profile id 0 is the default single-radio station, so it is the id
        // most likely to be reached for first and must not slip through as a falsy value.
        assert!(openable(Instance::Radio(0)).is_err(), "r0 is a real radio, not an absence");
    }

    // Unbound surfaces name no chain; only `r<id>` does. `ChainRef::ActiveRadio` means "follow
    // the active radio", the correct reading for the main window AND for a torn-off board.
    #[test]
    fn only_radio_bound_labels_name_a_chain() {
        assert_eq!(chain_of_label("main"), ChainRef::ActiveRadio);
        assert_eq!(chain_of_label("panel-waterfall"), ChainRef::ActiveRadio);
        assert_eq!(chain_of_label("panel-waterfall-w3"), ChainRef::ActiveRadio);
        assert_eq!(
            chain_of_label("panel-waterfall-r0"),
            ChainRef::Radio(0),
            "profile id 0 is a REAL radio, not an absence — the whole reason ChainRef exists"
        );
        assert_eq!(chain_of_label("panel-waterfall-r7"), ChainRef::Radio(7));
    }

    #[test]
    fn a_second_chain_is_refused_loudly_and_changes_nothing() {
        let mut chains = Chains::new(0, engine());
        let err = chains
            .add(1, engine())
            .expect_err("the cap must reject a second chain");
        // Not a silent no-op, not a truncation: the reason is in the text so a future caller
        // knows the refusal is deliberate.
        assert!(err.contains("HARD-CAPPED"), "{err}");
        assert!(err.contains("multi-radio-rollback.md"), "{err}");
        // The registry is untouched by the failed add.
        assert!(chains.get(1).is_none(), "the refused chain was not stored");
        assert!(chains.get(0).is_some(), "the existing chain was not evicted");
    }

    #[test]
    fn re_registering_an_existing_chain_is_refused_too() {
        let mut chains = Chains::new(0, engine());
        let err = chains
            .add(0, engine())
            .expect_err("replacing a live chain must not be silent");
        assert!(err.contains("already registered"), "{err}");
    }

    #[test]
    fn geometry_persists_only_for_stable_instances() {
        // `w<n>` ids are recycled as sub-panels are torn off, so a saved rect would be
        // inherited by a stranger.
        assert!(!Instance::Window(3).persists_geometry());
        assert!(Instance::Main.persists_geometry());
        assert!(Instance::Radio(2).persists_geometry());
    }
}
