// The text of files, fetched once each and shared by every code box and
// by the trace view.

import { useEffect, useState } from 'react'
import { fetchCode, type CodeSpan, type Target } from '../api'

/** Which text of a file: of which comparison, and before or after it. */
export interface Source {
  /** Null for the files as they are now. */
  target: Target | null
  side: 'head' | 'base'
}

export const NOW: Source = { target: null, side: 'head' }

const files = new Map<string, Promise<CodeSpan[][]>>()

export function loadCode(path: string, source: Source = NOW): Promise<CodeSpan[][]> {
  const key = `${JSON.stringify(source.target)}\n${source.side}\n${path}`
  let known = files.get(key)
  if (!known) {
    known = fetchCode(path, source.target, source.side)
    // A failed fetch is forgotten, so that it is tried again.
    known.catch(() => files.delete(key))
    files.set(key, known)
  }
  return known
}

/** Forgets every file: they may have changed on disk. */
export function forgetCode(): void {
  files.clear()
}

/** The lines of a file, or null until they have arrived. */
export function useCode(path: string | null, source: Source = NOW): CodeSpan[][] | null {
  const [loaded, setLoaded] = useState<{ path: string; source: Source; lines: CodeSpan[][] } | null>(null)
  useEffect(() => {
    if (path === null) return
    let current = true
    loadCode(path, source)
      .then((lines) => {
        if (current) setLoaded({ path, source, lines })
      })
      .catch(() => {})
    return () => {
      current = false
    }
  }, [path, source])
  return loaded && loaded.path === path && loaded.source === source ? loaded.lines : null
}
