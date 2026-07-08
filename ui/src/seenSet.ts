// A web-storage-backed "ids already celebrated" set, shared by the achievement +
// Journey unlock watchers. Persists to BOTH localStorage and sessionStorage and
// reads their UNION, so a write that fails in one store (quota / private mode)
// can't leave a stale set that re-toasts an already-celebrated unlock on reload.

function stores(): Storage[] {
  const out: Storage[] = []
  try {
    if (window.localStorage) out.push(window.localStorage)
  } catch {
    /* localStorage access blocked */
  }
  try {
    if (window.sessionStorage) out.push(window.sessionStorage)
  } catch {
    /* sessionStorage access blocked */
  }
  return out
}

/** The set of ids already celebrated for `key`, merged across backends.
 * `null` = never baselined anywhere (first ever run → baseline silently). */
export function readSeen(key: string): Set<string> | null {
  const ids = new Set<string>()
  let found = false
  for (const store of stores()) {
    try {
      const raw = store.getItem(key)
      if (raw != null) {
        found = true
        for (const id of JSON.parse(raw) as string[]) ids.add(id)
      }
    } catch {
      /* unreadable / malformed — skip this store */
    }
  }
  return found ? ids : null
}

export function writeSeen(key: string, seen: Set<string>): void {
  const raw = JSON.stringify([...seen])
  for (const store of stores()) {
    try {
      store.setItem(key, raw)
    } catch {
      /* full/unavailable — the other store (and the in-session ref) still hold */
    }
  }
}
