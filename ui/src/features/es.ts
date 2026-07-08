import { subsolarPoint } from '../mapGeo'

/** True during the boreal Sporadic-E season — mirrors the engine's is_es_season
 *  (solar declination > 15°, which is the subsolar latitude). A soft seasonal PRIOR
 *  only: it can suggest "watch 6m" but NEVER declares an opening — real Es status comes
 *  from the spot-evidenced openings list. */
export function isEsSeason(nowMs: number): boolean {
  return subsolarPoint(nowMs).lat > 15
}
