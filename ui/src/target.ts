// What is being reviewed, kept in the address so that a reload, a shared
// link, and the two pages (changes and map) all see the same.

import type { Target } from './api'

/** The target is kept in the address, so a reload or a shared link keeps it. */
export function readTarget(): Target {
  const params = new URLSearchParams(window.location.search)
  switch (params.get('mode')) {
    case 'staged':
      return { mode: 'staged' }
    case 'branch':
      return { mode: 'branch', base: params.get('base') ?? 'main' }
    case 'commit':
      return { mode: 'commit', commit: params.get('commit') ?? 'HEAD' }
    case 'range':
      return { mode: 'range', spec: params.get('spec') ?? '' }
    default:
      return { mode: 'working' }
  }
}

export function writeTarget(target: Target) {
  const params = new URLSearchParams()
  if (target.mode !== 'working') params.set('mode', target.mode)
  if (target.mode === 'branch') params.set('base', target.base)
  if (target.mode === 'commit') params.set('commit', target.commit)
  if (target.mode === 'range') params.set('spec', target.spec)
  const query = params.toString()
  // The part after `#` says which page is shown; it stays.
  const path = query === '' ? window.location.pathname : `?${query}`
  window.history.replaceState(null, '', path + window.location.hash)
}
