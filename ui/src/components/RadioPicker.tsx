import { Radio } from 'lucide-react'
import type { RadioLaunchInfo } from '../api'

/** First-screen launch picker for the two-radio setup: "which radio is this window?" Shown only
 *  when simultaneous-radios is enabled AND ≥2 radios are configured AND this window launched
 *  without a profile (see the backend `radio_launch_info`). Choosing a radio relaunches the
 *  window bound to it — so the operator never touches a shortcut or environment variable. A radio
 *  already open in another window is greyed out, so you can't accidentally double-drive one. */
export function RadioPicker({
  info,
  onChoose,
}: {
  info: RadioLaunchInfo
  onChoose: (id: number) => void
}) {
  return (
    <div className="radio-picker-overlay" role="dialog" aria-modal="true" aria-label="Choose radio">
      <div className="radio-picker">
        <div className="radio-picker-head">
          <Radio size={22} aria-hidden="true" />
          <h1>Which radio?</h1>
        </div>
        <p className="radio-picker-sub">
          You have two radios running at once. Pick the radio this window will operate — you can
          open a second window for the other. They share one logbook.
        </p>
        <div className="radio-picker-list">
          {info.radios.map((r) => (
            <button
              key={r.id}
              type="button"
              className={`radio-picker-btn${r.inUse ? ' in-use' : ''}`}
              disabled={r.inUse}
              title={r.inUse ? `${r.name} is already open in another window` : `Operate ${r.name}`}
              onClick={() => onChoose(r.id)}
            >
              <span className="radio-picker-name">{r.name}</span>
              {r.inUse && <span className="radio-picker-tag">in use</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}
