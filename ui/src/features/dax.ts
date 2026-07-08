// SmartSDR DAX virtual-audio detection — shared by the Settings network-rig
// section and the setup wizard's rig step (one matcher, no copy-paste drift).
// DAX devices are plain Windows sound devices ("DAX Audio RX 1", "DAX Audio TX");
// when both sides exist, one click pairs them as Nexus's audio in/out.

export interface DaxPair {
  input: string
  output: string
}

/** Find the DAX receive/transmit devices in the system device lists, preferring
 * RX 1 (the slice-A stream) when several DAX RX channels exist. Null when
 * either side is missing — the pairing affordance simply doesn't render.
 *
 * TX preference order (operator-verified on a real 6400M): newer FlexRadio DAX
 * drivers expose the LIVE transmit endpoint as bare "DAX TX (FlexRadio DAX)"
 * while an endpoint named "DAX Audio TX" also enumerates — the bare name is
 * the one that actually keys audio. Older installs have only "DAX Audio TX",
 * which stays as the fallback (it's the classic docs' device). */
export function findDaxDevices(input: string[], output: string[]): DaxPair | null {
  const rx = input.find((d) => /dax.*rx\s*1/i.test(d)) ?? input.find((d) => /dax/i.test(d))
  const tx =
    output.find((d) => /\bdax\s+tx\b/i.test(d)) ??
    output.find((d) => /dax.*tx/i.test(d)) ??
    output.find((d) => /dax/i.test(d))
  return rx && tx ? { input: rx, output: tx } : null
}

/** True when the operator's current audio choice is already DAX on both sides —
 * the pair button is a BOOTSTRAPPER, not an enforcer: once any DAX devices are
 * chosen (auto or by hand), it must stop offering to "fix" them (the operator's
 * manual pick of a different DAX endpoint is ground truth, not drift). */
export function isDaxPaired(audioIn: string, audioOut: string): boolean {
  return /dax/i.test(audioIn) && /dax/i.test(audioOut)
}
