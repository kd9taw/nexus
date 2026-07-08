<!--
Thanks for contributing to Tempo. Please complete the sections below.
Keep changes focused and honest about what was and wasn't validated.
-->

## Summary

<!-- What does this PR change, and why? Keep it concise. -->

## Linked issue

<!-- e.g. "Closes #123" or "Refs #123". Use "n/a" if there is no associated issue. -->

Closes #

## Checklist

- [ ] `cargo test` passes on the workspace (headless modem + engine + net + DX1 round-trips).
- [ ] `cargo clippy --all-targets` is clean (no new warnings).
- [ ] UI touched? `npm --prefix ui run build` (`tsc -b && vite build`) passes. (Skip if no UI change.)
- [ ] Docs updated where relevant (README, WINDOWS.md, code comments, DTO contract in `tempo-app/src/dto.rs` ↔ `ui/`).
- [ ] **Validation honesty:** I have stated below what this change was validated against.

## Validation

<!--
Be explicit and honest. Tempo's waveforms are simulation-validated, NOT yet on-air.
State which applies:
  - Simulation / loopback only (AWGN / fading sweeps, headless round-trips), OR
  - Bench / loopback hardware, OR
  - On-air (state band, rig, conditions, and the other station).
Do not imply on-air proof for changes that were only simulated.
-->

- **Validated against:**
- **Notes:**

## License & sign-off

By submitting this pull request, I agree that my contributions are licensed under
**GPL-3.0-or-later** (consistent with the rest of Tempo; see `COPYING`), and I have
signed off on the [Developer Certificate of Origin](https://developercertificate.org/)
by adding a `Signed-off-by:` line to my commits (`git commit -s`).

- [ ] My commits are signed off (DCO) and my contribution is GPL-3.0-or-later.
