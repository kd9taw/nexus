// Continent×Band live-activity matrix (DXHeat-style), operator-anchored — "which
// band×region is alive FOR YOU, in the last window," not the global firehose. Cell
// intensity = distinct anchored stations, colored by the existing inferno heatColor LUT.
import { heatColor } from '../../propViz'
import type { RegionBandCell } from '../../types'

const BAND_ORDER = ['160m', '80m', '60m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m', '4m', '2m']

export function ActivityMatrix({
  cells,
  onBandClick,
  activeBand,
}: {
  cells: RegionBandCell[]
  onBandClick?: (band: string) => void
  activeBand?: string | null
}) {
  const regions = [...new Set(cells.map((c) => c.region))].sort()
  const bands = BAND_ORDER.filter((b) => cells.some((c) => c.band === b))
  const max = Math.max(1, ...cells.map((c) => c.stations))
  const at = (band: string, region: string) => cells.find((c) => c.band === band && c.region === region)
  return (
    <table className="amx">
      <thead>
        <tr>
          <th className="amx-corner" aria-label="band" />
          {regions.map((r) => (
            <th key={r} className="amx-region">
              {r}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {bands.map((band) => (
          <tr key={band} className={activeBand === band ? 'is-active' : ''}>
            <th
              className={`amx-band${onBandClick ? ' is-clickable' : ''}`}
              onClick={onBandClick ? () => onBandClick(band) : undefined}
              title={onBandClick ? `Focus ${band} on the map` : undefined}
            >
              {band}
            </th>
            {regions.map((region) => {
              const cell = at(band, region)
              const n = cell?.stations ?? 0
              return (
                <td
                  key={region}
                  className="amx-cell"
                  style={n > 0 ? { background: heatColor(n / max) } : undefined}
                  title={
                    cell
                      ? `${region} ${band}: ${n} stn${n === 1 ? '' : 's'} (${cell.hearMe} hear you, ${cell.iHear} you hear)`
                      : `${region} ${band}: —`
                  }
                >
                  {n > 0 ? n : ''}
                </td>
              )
            })}
          </tr>
        ))}
      </tbody>
    </table>
  )
}
