// One scope with everything around it: its code in the middle, the scopes
// that use it on the left, the scopes it uses on the right, and a line from
// each name in the code to what it refers to.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { fetchNode, type NodeInfo, type Place, type Use } from '../api'
import { useCode, type Source } from './code.ts'

interface Props {
  id: string
  /** Which files: as they are now, or the side after a change. */
  source: Source
  onOpen(id: string): void
  onShowOnMap(id: string): void
  onClose(): void
}

interface Curve {
  key: string
  d: string
  side: 'in' | 'out'
}

/** A line of source with the referring name marked. */
function PlaceLine({ place, anchor }: { place: Place; anchor?: string }) {
  const [start, end] = [place.column - place.indent, place.end_column - place.indent]
  return (
    <code className="mtrace-place">
      <span className="mtrace-lineno">{place.line}</span>
      {place.text.slice(0, start)}
      <mark data-anchor={anchor}>{place.text.slice(start, end)}</mark>
      {place.text.slice(end)}
    </code>
  )
}

function UseCard({ use, side, index, onOpen }: { use: Use; side: 'in' | 'out'; index: number; onOpen(id: string): void }) {
  return (
    <li>
      <button type="button" className="mtrace-card" data-card={`${side}:${index}`} onClick={() => onOpen(use.id)} title={`Trace ${use.id}`}>
        <span className="mtrace-card-name">
          <span className="mword">{use.lang_kind} </span>
          {use.label}
          {use.count > 1 && <span className="minspect-times"> ×{use.count}</span>}
        </span>
        <span className="mtrace-card-path">{use.path}</span>
        {/* A caller shows the lines where it uses the scope in the middle. */}
        {side === 'in' &&
          use.places.slice(0, 3).map((place, position) => (
            <PlaceLine key={position} place={place} anchor={position === 0 ? `in:${index}` : undefined} />
          ))}
      </button>
    </li>
  )
}

export function Trace({ id, source, onOpen, onShowOnMap, onClose }: Props) {
  const [info, setInfo] = useState<NodeInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [curves, setCurves] = useState<Curve[]>([])
  const content = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let current = true
    fetchNode(id, 'code', source.target)
      .then((loaded) => {
        if (!current) return
        setInfo(loaded)
        setError(null)
      })
      .catch((problem: unknown) => {
        if (current) setError(problem instanceof Error ? problem.message : String(problem))
      })
    return () => {
      current = false
    }
  }, [id, source])

  const shown = info && info.id === id ? info : null
  // A scope that is only there before the change is read from that side.
  const side = useMemo((): Source => (shown?.side === 'base' ? { target: source.target, side: 'base' } : source), [shown?.side, source])
  const file = useCode(shown && shown.first_line > 0 ? shown.path : null, side)
  const MOST = 3000
  const lines = shown && file ? file.slice(shown.first_line - 1, Math.min(shown.last_line, shown.first_line - 1 + MOST)) : []

  // The names in the code that refer to something on the right, by line.
  const marks = new Map<number, { place: Place; key: string }[]>()
  shown?.dependencies.forEach((use, index) => {
    use.places.forEach((place, position) => {
      if (place.path !== shown.path) return
      const list = marks.get(place.line) ?? []
      list.push({ place, key: `out:${index}:${position}` })
      marks.set(place.line, list)
    })
  })

  /** Draws a curve from each marked name to its card, measured from the page. */
  const measure = useCallback(() => {
    const root = content.current
    if (!root) return
    const origin = root.getBoundingClientRect()
    const at = (element: Element, edge: 'left' | 'right') => {
      const rect = element.getBoundingClientRect()
      return {
        x: (edge === 'left' ? rect.left : rect.right) - origin.left,
        y: rect.top + rect.height / 2 - origin.top,
      }
    }
    const curve = (from: { x: number; y: number }, to: { x: number; y: number }) => {
      const bend = Math.max(30, (to.x - from.x) / 2)
      return `M${from.x} ${from.y}C${from.x + bend} ${from.y} ${to.x - bend} ${to.y} ${to.x} ${to.y}`
    }
    const next: Curve[] = []
    for (const mark of root.querySelectorAll('[data-anchor^="out:"]')) {
      const [, index] = mark.getAttribute('data-anchor')!.split(':')
      const card = root.querySelector(`[data-card="out:${index}"]`)
      if (card) next.push({ key: mark.getAttribute('data-anchor')!, d: curve(at(mark, 'right'), at(card, 'left')), side: 'out' })
    }
    const head = root.querySelector('[data-head]')
    for (const mark of root.querySelectorAll('[data-anchor^="in:"]')) {
      if (head) next.push({ key: mark.getAttribute('data-anchor')!, d: curve(at(mark, 'right'), at(head, 'left')), side: 'in' })
    }
    setCurves(next)
  }, [])

  useLayoutEffect(() => {
    measure()
    const root = content.current
    if (!root) return
    const observer = new ResizeObserver(measure)
    observer.observe(root)
    return () => observer.disconnect()
  }, [measure, shown, file])

  return (
    <div className="mtrace" role="dialog" aria-label="Trace">
      <header className="mtrace-bar">
        <strong>Trace</strong>
        {shown && (
          <span>
            <span className="mword">{shown.lang_kind}</span> {shown.label}
            <span className="mtrace-where">
              {' '}
              {shown.path}
              {shown.first_line > 0 && `:${shown.first_line}`}
            </span>
          </span>
        )}
        <span className="mtrace-spacer" />
        <button type="button" onClick={() => onShowOnMap(id)}>Show on map</button>
        <button type="button" onClick={onClose} title="Back to the map (Esc)">Close</button>
      </header>
      {error && <p className="map-error">{error}</p>}
      {shown && (
        <div className="mtrace-scroll">
          <div className="mtrace-content" ref={content}>
            <section className="mtrace-side">
              <h3>Used by <span className="minspect-count">{shown.callers.length}</span></h3>
              {shown.callers.length === 0 && <p className="minspect-empty">Nothing in this repository uses it.</p>}
              <ul>
                {shown.callers.map((use, index) => (
                  <UseCard key={use.id} use={use} side="in" index={index} onOpen={onOpen} />
                ))}
              </ul>
            </section>
            <section className="mtrace-code">
              <h3 data-head>
                <span className="mword">{shown.lang_kind}</span> {shown.label}
              </h3>
              {shown.first_line === 0 && <p className="minspect-empty">A directory has no code of its own.</p>}
              <pre>
                {lines.map((line, row) => {
                  const number = shown.first_line + row
                  return (
                    <div key={number} className="mtrace-line">
                      <span className="mtrace-lineno">{number}</span>
                      <span className="mtrace-text">
                        {line.length === 0
                          ? ' '
                          : line.map((span, position) => (
                              <span key={position} className={span.class ? `tok-${span.class}` : undefined}>
                                {span.text}
                              </span>
                            ))}
                        {/* Marks sit over the names, placed by column: the font is monospace. */}
                        {marks.get(number)?.map(({ place, key }) => (
                          <span
                            key={key}
                            className="mtrace-mark"
                            data-anchor={key}
                            style={{ left: `${place.column}ch`, width: `${Math.max(1, place.end_column - place.column)}ch` }}
                          />
                        ))}
                      </span>
                    </div>
                  )
                })}
              </pre>
              {shown.last_line - shown.first_line + 1 > MOST && (
                <p className="minspect-empty">The first {MOST} lines of {shown.last_line - shown.first_line + 1}.</p>
              )}
            </section>
            <section className="mtrace-side">
              <h3>Uses <span className="minspect-count">{shown.dependencies.length}</span></h3>
              {shown.dependencies.length === 0 && <p className="minspect-empty">It uses nothing in this repository.</p>}
              <ul>
                {shown.dependencies.map((use, index) => (
                  <UseCard key={use.id} use={use} side="out" index={index} onOpen={onOpen} />
                ))}
              </ul>
            </section>
            <svg className="mtrace-curves" aria-hidden="true">
              {curves.map((curve) => (
                <path key={curve.key} className={`mtrace-curve mtrace-curve-${curve.side}`} d={curve.d} />
              ))}
            </svg>
          </div>
        </div>
      )}
    </div>
  )
}
